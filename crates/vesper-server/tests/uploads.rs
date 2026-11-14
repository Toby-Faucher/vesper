//! Uploads over the in-process router: limits, quota, the served headers and who may download.

mod common;

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use axum::body::{Body, Bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use common::{ORIGIN, Session, TestServer, get, post, request, spawn_sfu};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use vesper_protocol::{AttachmentKind, Role, UploadDone};
use vesper_server::router;
use vesper_server::uploads::{self, MAX_UPLOAD, STALE_MS};
use vesper_store::now_ms;

const PNG_HEX: &str = "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d49444154789c63f8cfc0f01f00050001ff89993d1d0000000049454e44ae426082";

fn png() -> Vec<u8> {
    (0..PNG_HEX.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&PNG_HEX[i..i + 2], 16).unwrap())
        .collect()
}

/// One request through the router, with the raw response body.
async fn raw(server: &TestServer, mut req: Request<Body>) -> (StatusCode, HeaderMap, Bytes) {
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
    let res = router(server.state.clone()).oneshot(req).await.unwrap();
    let (parts, body) = res.into_parts();
    (
        parts.status,
        parts.headers,
        body.collect().await.unwrap().to_bytes(),
    )
}

fn upload_req(room: i64, name: &str, body: Body, s: &Session) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(format!("/api/rooms/{room}/uploads?name={name}"))
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, &s.cookie)
        .header(header::CONTENT_TYPE, "image/png")
        .body(body)
        .unwrap()
}

/// Uploads `bytes` (with a Content-Length); returns the status and the JSON body.
async fn upload(
    server: &TestServer,
    s: &Session,
    room: i64,
    name: &str,
    bytes: impl Into<Body>,
) -> (StatusCode, Value) {
    let (status, _, v) = server.call(upload_req(room, name, bytes.into(), s)).await;
    (status, v)
}

/// Uploads and expects 201; returns the upload id.
async fn upload_ok(server: &TestServer, s: &Session, room: i64, bytes: &[u8]) -> String {
    let (status, v) = upload(server, s, room, "f.bin", bytes.to_vec()).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let done: UploadDone = serde_json::from_value(v).unwrap();
    done.attachment.id
}

async fn download(server: &TestServer, id: &str, s: &Session) -> (StatusCode, HeaderMap, Bytes) {
    raw(server, get(&format!("/api/uploads/{id}"), s)).await
}

/// Posts a message in `room` with `attachments`; returns the status and body.
async fn attach(server: &TestServer, s: &Session, room: i64, ids: &[&str]) -> (StatusCode, Value) {
    let (status, _, v) = server
        .call(post(
            &format!("/api/rooms/{room}/messages"),
            json!({ "body": "files", "attachments": ids }),
            s,
        ))
        .await;
    (status, v)
}

async fn general(server: &TestServer) -> i64 {
    server.state.store.rooms().await.unwrap()[0].id
}

/// The owner makes a restricted room listing `members`.
async fn restricted(server: &TestServer, owner: &Session, members: &[&Session]) -> i64 {
    let (status, _, v) = server
        .call(post("/api/rooms", json!({ "name": "secret" }), owner))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["id"].as_i64().unwrap();
    let ids: Vec<&str> = members.iter().map(|s| s.user.user_id.as_str()).collect();
    for (method, path, body) in [
        (
            Method::PUT,
            format!("/api/rooms/{id}/members"),
            json!({ "user_ids": ids }),
        ),
        (
            Method::PATCH,
            format!("/api/rooms/{id}"),
            json!({ "restricted": true }),
        ),
    ] {
        let (status, _, v) = server
            .call(request(method, &path, Some(&body), Some(&owner.cookie)))
            .await;
        assert_eq!(status, StatusCode::OK, "{v}");
    }
    id
}

fn entries(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|d| {
            d.map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect()
        })
        .unwrap_or_default()
}

fn tmp_dir(server: &TestServer) -> std::path::PathBuf {
    server.data.path().join("uploads/tmp")
}

fn file_of(server: &TestServer, id: &str) -> std::path::PathBuf {
    uploads::upload_path(server.data.path(), id).unwrap()
}

fn h<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .unwrap_or_else(|| panic!("no {name}"))
        .to_str()
        .unwrap()
}

/// The headers every served upload carries.
fn assert_safe_headers(headers: &HeaderMap) {
    assert_eq!(h(headers, "x-content-type-options"), "nosniff");
    assert_eq!(
        h(headers, "content-security-policy"),
        "default-src 'none'; sandbox"
    );
    assert_eq!(h(headers, "cross-origin-resource-policy"), "same-origin");
    assert_eq!(
        h(headers, "cache-control"),
        "private, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn upload_id_traversal_is_404() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Owner).await;
    let id = upload_ok(&server, &alice, general(&server).await, b"secret").await;
    let (_, _, unknown) = download(&server, &"0".repeat(32), &alice).await;
    for bad in [
        "..%2F..%2Fvesper.db".to_string(),
        "%2e%2e%2f%2e%2e%2fvesper.db".into(),
        "%2Fetc%2Fpasswd".into(),
        id.to_uppercase(),
        format!("{}%00", &id[..31]),
        format!("{id}%00"),
        format!("{id}0"),
        id[..31].to_string(),
        "A".repeat(32),
        format!("..%2F{}", &id[..29]),
    ] {
        let (status, _, body) = download(&server, &bad, &alice).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{bad}");
        assert_eq!(body, unknown, "{bad}");
    }
    let (status, _, body) = download(&server, &id, &alice).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&body[..], b"secret");
}

#[tokio::test]
async fn png_is_inline_image() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let room = general(&server).await;
    let (status, v) = upload(&server, &alice, room, "cat.png", png()).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let a = serde_json::from_value::<UploadDone>(v).unwrap().attachment;
    assert_eq!(
        (a.name.as_str(), a.mime.as_str(), a.kind),
        ("cat.png", "image/png", AttachmentKind::Image)
    );
    assert_eq!(a.size, png().len() as i64);
    let (status, headers, body) = download(&server, &a.id, &alice).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.to_vec(), png());
    assert_eq!(h(&headers, "content-type"), "image/png");
    assert_eq!(
        h(&headers, "content-disposition"),
        "inline; filename=\"cat.png\"; filename*=UTF-8''cat.png"
    );
    assert_safe_headers(&headers);

    // On disk: named by the id, file 0600, dirs 0700, nothing left in tmp.
    let path = file_of(&server, &a.id);
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    let uploads = server.data.path().join("uploads");
    for dir in [&uploads, path.parent().unwrap(), &tmp_dir(&server)] {
        assert_eq!(mode(dir), 0o700, "{}", dir.display());
    }
    assert_eq!(entries(path.parent().unwrap()), std::slice::from_ref(&a.id));
    assert!(entries(&tmp_dir(&server)).is_empty());

    // A range request is served too.
    let mut req = get(&format!("/api/uploads/{}", a.id), &alice);
    req.headers_mut()
        .insert(header::RANGE, "bytes=0-7".parse().unwrap());
    let (status, headers, body) = raw(&server, req).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(&body[..], &png()[..8]);
    assert_eq!(h(&headers, "content-type"), "image/png");
    assert_safe_headers(&headers);
}

/// The served type follows the bytes, never the claimed type or the name.
#[tokio::test]
async fn html_disguised_as_png_is_attachment() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let room = general(&server).await;
    for (name, bytes) in [
        ("cat.png", &b"<html><script>alert(1)</script>"[..]),
        (
            "x.svg",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>",
        ),
        // A PNG signature cut short is not a PNG.
        ("short.png", &png()[..7]),
        ("empty.png", b""),
    ] {
        let (status, v) = upload(&server, &alice, room, name, bytes.to_vec()).await;
        assert_eq!(status, StatusCode::CREATED, "{v}");
        let a = serde_json::from_value::<UploadDone>(v).unwrap().attachment;
        assert_eq!(a.mime, "application/octet-stream", "{name}");
        assert_eq!(a.kind, AttachmentKind::File, "{name}");
        let (status, headers, body) = download(&server, &a.id, &alice).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(&body[..], bytes);
        assert_eq!(h(&headers, "content-type"), "application/octet-stream");
        assert!(
            h(&headers, "content-disposition")
                .starts_with(&format!("attachment; filename=\"{name}\"")),
            "{name}"
        );
        assert_safe_headers(&headers);
    }
}

#[tokio::test]
async fn svg_is_attachment() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let svg =
        b"<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\" onload=\"alert(1)\"/>";
    let (status, v) = upload(
        &server,
        &alice,
        general(&server).await,
        "logo.svg",
        svg.to_vec(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let a = serde_json::from_value::<UploadDone>(v).unwrap().attachment;
    assert_eq!(a.kind, AttachmentKind::File);
    let (_, headers, _) = download(&server, &a.id, &alice).await;
    assert_eq!(h(&headers, "content-type"), "application/octet-stream");
    assert!(h(&headers, "content-disposition").starts_with("attachment;"));
    assert_safe_headers(&headers);
}

/// Audio and video are served inline with their sniffed type, the same safe headers, and ranges
/// so a player can seek. The name and the claimed type play no part.
#[tokio::test]
async fn media_is_inline() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let room = general(&server).await;
    for (name, head, mime, kind) in [
        (
            "clip.txt",
            &b"\0\0\0\x18ftypisom"[..],
            "video/mp4",
            AttachmentKind::Video,
        ),
        (
            "song.png",
            b"ID3\x04\0\0\0\0",
            "audio/mpeg",
            AttachmentKind::Audio,
        ),
    ] {
        let bytes = [head, &[0u8; 64][..]].concat();
        let (status, v) = upload(&server, &alice, room, name, bytes.clone()).await;
        assert_eq!(status, StatusCode::CREATED, "{v}");
        let a = serde_json::from_value::<UploadDone>(v).unwrap().attachment;
        assert_eq!((a.mime.as_str(), a.kind), (mime, kind), "{name}");
        let mut req = get(&format!("/api/uploads/{}", a.id), &alice);
        req.headers_mut()
            .insert(header::RANGE, "bytes=4-11".parse().unwrap());
        let (status, headers, body) = raw(&server, req).await;
        assert_eq!(status, StatusCode::PARTIAL_CONTENT, "{name}");
        assert_eq!(&body[..], &bytes[4..12], "{name}");
        assert_eq!(h(&headers, "content-type"), mime);
        assert!(h(&headers, "content-disposition").starts_with("inline;"));
        assert_safe_headers(&headers);
    }
}

#[tokio::test]
async fn file_name_cannot_inject_headers() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let room = general(&server).await;
    // a"b CR LF Set-Cookie: x=1; ../ RLO "gpj.exe", percent-encoded in the query.
    let name = "a%22b%0D%0ASet-Cookie%3A%20x%3D1%3B%20..%2F%E2%80%AEgpj.exe";
    let (status, v) = upload(&server, &alice, room, name, b"x".to_vec()).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let a = serde_json::from_value::<UploadDone>(v).unwrap().attachment;
    assert_eq!(a.name, "a\"bSet-Cookie: x=1; .._gpj.exe");
    let (_, headers, _) = download(&server, &a.id, &alice).await;
    assert!(headers.get(header::SET_COOKIE).is_none());
    let cd = h(&headers, "content-disposition");
    assert_eq!(
        cd,
        "attachment; filename=\"a_bSet-Cookie: x=1; .._gpj.exe\"; \
         filename*=UTF-8''a%22bSet-Cookie%3A%20x%3D1%3B%20.._gpj.exe"
    );
    // Unicode survives in filename*.
    let (_, v) = upload(&server, &alice, room, "%C3%A9t%C3%A9.txt", b"x".to_vec()).await;
    let a = serde_json::from_value::<UploadDone>(v).unwrap().attachment;
    assert_eq!(a.name, "été.txt");
    let (_, headers, _) = download(&server, &a.id, &alice).await;
    assert!(h(&headers, "content-disposition").ends_with("filename*=UTF-8''%C3%A9t%C3%A9.txt"));
}

#[tokio::test]
async fn oversize_body_is_413_and_cleaned() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let room = general(&server).await;
    let mib = Bytes::from(vec![7u8; 1 << 20]);
    let chunks = |extra: usize| {
        let mut c: Vec<Result<Bytes, std::io::Error>> = (0..25).map(|_| Ok(mib.clone())).collect();
        c.push(Ok(Bytes::from(vec![7u8; extra])));
        Body::from_stream(futures_util::stream::iter(c))
    };
    // Chunked (no Content-Length), one byte over.
    let req = upload_req(room, "big.bin", chunks(1), &alice);
    assert!(req.headers().get(header::CONTENT_LENGTH).is_none());
    let (status, _, v) = server.call(req).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{v}");
    assert_eq!(v["code"], "too_large");
    assert!(entries(&tmp_dir(&server)).is_empty());
    // A declared length over the cap is refused before the body is read.
    let mut req = upload_req(room, "big.bin", Body::from("tiny"), &alice);
    req.headers_mut().insert(
        header::CONTENT_LENGTH,
        (MAX_UPLOAD + 1).to_string().parse().unwrap(),
    );
    let (status, _, _) = server.call(req).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(entries(&tmp_dir(&server)).is_empty());
    assert_eq!(server.state.store.upload_bytes().await.unwrap(), 0);
    // Exactly the cap is fine.
    let (status, _, v) = server
        .call(upload_req(room, "max.bin", chunks(0), &alice))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["attachment"]["size"], MAX_UPLOAD);
}

#[tokio::test]
async fn interrupted_body_is_cleaned() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let body = Body::from_stream(futures_util::stream::iter(vec![
        Ok(Bytes::from_static(b"start")),
        Err(std::io::Error::other("client went away")),
    ]));
    let (status, _, _) = server
        .call(upload_req(general(&server).await, "x", body, &alice))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(entries(&tmp_dir(&server)).is_empty());
    assert_eq!(server.state.store.upload_bytes().await.unwrap(), 0);
}

const KIB: usize = 1024;

async fn quota_server() -> TestServer {
    TestServer::with(|c| c.upload_quota_bytes = 1 << 20, spawn_sfu()).await
}

#[tokio::test]
async fn quota_full_is_507() {
    let server = quota_server().await;
    let alice = server.account("alice", Role::Member).await;
    let room = general(&server).await;
    upload_ok(&server, &alice, room, &vec![1; 600 * KIB]).await;
    let (status, v) = upload(&server, &alice, room, "b", vec![1; 600 * KIB]).await;
    assert_eq!(status, StatusCode::INSUFFICIENT_STORAGE, "{v}");
    assert_eq!(v["code"], "quota_full");
    assert!(entries(&tmp_dir(&server)).is_empty());
    assert_eq!(
        server.state.store.upload_bytes().await.unwrap(),
        600 * KIB as i64
    );
    // What still fits is taken.
    upload_ok(&server, &alice, room, &vec![1; 424 * KIB]).await;
    let (status, _) = upload(&server, &alice, room, "c", vec![1u8]).await;
    assert_eq!(status, StatusCode::INSUFFICIENT_STORAGE);
}

/// Ten 300 KiB uploads at once against a 1 MiB quota: exactly three fit.
#[tokio::test]
async fn concurrent_uploads_cannot_pass_the_quota() {
    let server = quota_server().await;
    let room = general(&server).await;
    let a = server.account("alice", Role::Member).await;
    let b = server.account("bob", Role::Member).await;
    let body = vec![1u8; 300 * KIB];
    let calls = (0..10).map(|i| {
        let s = if i % 2 == 0 { &a } else { &b };
        upload(&server, s, room, "f", body.clone())
    });
    let results = futures_util::future::join_all(calls).await;
    let ok = results
        .iter()
        .filter(|r| r.0 == StatusCode::CREATED)
        .count();
    let full = results
        .iter()
        .filter(|r| r.0 == StatusCode::INSUFFICIENT_STORAGE)
        .count();
    assert_eq!((ok, full), (3, 7), "{results:?}");
    assert_eq!(
        server.state.store.upload_bytes().await.unwrap(),
        900 * KIB as i64
    );
    assert!(entries(&tmp_dir(&server)).is_empty());
}

#[tokio::test]
async fn uploads_are_rate_limited_per_user() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let a = server.account("alice", Role::Member).await;
    let b = server.account("bob", Role::Member).await;
    for _ in 0..10 {
        upload_ok(&server, &a, room, b"x").await;
    }
    let (status, v) = upload(&server, &a, room, "x", b"x".to_vec()).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{v}");
    upload_ok(&server, &b, room, b"x").await;
}

#[tokio::test]
async fn upload_needs_session_origin_and_a_visible_room() {
    let server = TestServer::new().await;
    let owner = server.account("owner", Role::Owner).await;
    let bob = server.account("bob", Role::Member).await;
    let secret = restricted(&server, &owner, &[]).await;
    let (status, v) = upload(&server, &bob, secret, "x", b"x".to_vec()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");
    let (status, _) = upload(&server, &bob, 9999, "x", b"x".to_vec()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let mut req = upload_req(general(&server).await, "x", Body::from("x"), &bob);
    req.headers_mut().remove(header::COOKIE);
    assert_eq!(server.call(req).await.0, StatusCode::UNAUTHORIZED);
    let mut req = upload_req(general(&server).await, "x", Body::from("x"), &bob);
    req.headers_mut()
        .insert(header::ORIGIN, "https://evil.example".parse().unwrap());
    assert_eq!(server.call(req).await.0, StatusCode::FORBIDDEN);
    assert_eq!(server.state.store.upload_bytes().await.unwrap(), 0);
}

/// Who downloads what: attached files follow the room, unattached ones only the uploader.
#[tokio::test]
async fn other_room_upload_is_404() {
    let server = TestServer::new().await;
    let owner = server.account("owner", Role::Owner).await;
    let moder = server.account("moder", Role::Moderator).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let open = general(&server).await;
    let secret = restricted(&server, &owner, &[&alice]).await;
    let (_, _, unknown) = download(&server, &"0".repeat(32), &bob).await;
    let check = async |id: &str, who: &[(&Session, StatusCode)]| {
        for (s, want) in who {
            let (status, _, body) = download(&server, id, s).await;
            assert_eq!(status, *want, "{} on {id}", s.user.username);
            if *want == StatusCode::NOT_FOUND {
                assert_eq!(body, unknown);
            }
        }
    };
    use StatusCode as S;

    // Unattached: only the uploader, not even the owner.
    let draft = upload_ok(&server, &alice, secret, b"draft").await;
    check(
        &draft,
        &[
            (&alice, S::OK),
            (&bob, S::NOT_FOUND),
            (&moder, S::NOT_FOUND),
            (&owner, S::NOT_FOUND),
        ],
    )
    .await;
    let open_draft = upload_ok(&server, &alice, open, b"draft").await;
    check(
        &open_draft,
        &[
            (&alice, S::OK),
            (&bob, S::NOT_FOUND),
            (&owner, S::NOT_FOUND),
        ],
    )
    .await;

    // Attaching needs the uploader, the same room and an unattached upload.
    assert_eq!(
        attach(&server, &owner, secret, &[&draft]).await.0,
        S::CONFLICT
    );
    assert_eq!(
        attach(&server, &alice, open, &[&draft]).await.0,
        S::CONFLICT
    );
    let (status, v) = attach(&server, &alice, secret, &[&draft]).await;
    assert_eq!(status, S::CREATED, "{v}");
    assert_eq!(v["attachments"][0]["id"], draft.as_str());
    assert_eq!(
        attach(&server, &alice, secret, &[&draft]).await.0,
        S::CONFLICT
    );
    check(
        &draft,
        &[
            (&alice, S::OK),
            (&bob, S::NOT_FOUND),
            (&moder, S::OK),
            (&owner, S::OK),
        ],
    )
    .await;

    assert_eq!(
        attach(&server, &alice, open, &[&open_draft]).await.0,
        S::CREATED
    );
    check(
        &open_draft,
        &[
            (&alice, S::OK),
            (&bob, S::OK),
            (&moder, S::OK),
            (&owner, S::OK),
        ],
    )
    .await;

    // Taken off the list: gone for alice too.
    let (status, _, _) = server
        .call(request(
            Method::PUT,
            &format!("/api/rooms/{secret}/members"),
            Some(&json!({ "user_ids": [bob.user.user_id] })),
            Some(&owner.cookie),
        ))
        .await;
    assert_eq!(status, S::OK);
    check(&draft, &[(&alice, S::NOT_FOUND), (&bob, S::OK)]).await;

    // No session: 401.
    let req = request(Method::GET, &format!("/api/uploads/{draft}"), None, None);
    assert_eq!(raw(&server, req).await.0, S::UNAUTHORIZED);
}

#[tokio::test]
async fn deleting_message_removes_file() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let moder = server.account("moder", Role::Moderator).await;
    let room = general(&server).await;
    let one = upload_ok(&server, &alice, room, &png()).await;
    let two = upload_ok(&server, &alice, room, b"two").await;
    let (status, v) = attach(&server, &alice, room, &[&one, &two]).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let msg = v["id"].as_i64().unwrap();
    assert!(file_of(&server, &one).exists() && file_of(&server, &two).exists());
    let (status, _, _) = server
        .call(request(
            Method::DELETE,
            &format!("/api/messages/{msg}"),
            None,
            Some(&moder.cookie),
        ))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    for id in [&one, &two] {
        assert!(!file_of(&server, id).exists());
        assert_eq!(download(&server, id, &alice).await.0, StatusCode::NOT_FOUND);
    }
    assert_eq!(server.state.store.upload_bytes().await.unwrap(), 0);
}

#[tokio::test]
async fn stale_unattached_upload_is_swept() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let room = general(&server).await;
    let stale = upload_ok(&server, &alice, room, b"stale").await;
    let kept = upload_ok(&server, &alice, room, b"kept").await;
    assert_eq!(
        attach(&server, &alice, room, &[&kept]).await.0,
        StatusCode::CREATED
    );
    let now = now_ms();
    assert_eq!(
        uploads::sweep(&server.state, now + STALE_MS - 3_600_000)
            .await
            .unwrap(),
        0
    );
    assert!(file_of(&server, &stale).exists());
    assert_eq!(
        uploads::sweep(&server.state, now + 25 * 3_600_000)
            .await
            .unwrap(),
        1
    );
    assert!(!file_of(&server, &stale).exists());
    assert!(server.state.store.upload(&stale).await.is_err());
    assert_eq!(
        download(&server, &stale, &alice).await.0,
        StatusCode::NOT_FOUND
    );
    assert!(file_of(&server, &kept).exists());
    assert_eq!(download(&server, &kept, &alice).await.0, StatusCode::OK);
}

#[tokio::test]
async fn start_empties_tmp() {
    let server = TestServer::new().await;
    let tmp = tmp_dir(&server);
    std::fs::write(tmp.join("leftover"), b"x").unwrap();
    uploads::clean_tmp(server.data.path()).unwrap();
    assert!(entries(&tmp).is_empty());
}

#[tokio::test]
async fn disk_use_is_owner_only() {
    let server = TestServer::with(|c| c.upload_quota_bytes = 10 * 1024, spawn_sfu()).await;
    let owner = server.account("owner", Role::Owner).await;
    let moder = server.account("moder", Role::Moderator).await;
    let alice = server.account("alice", Role::Member).await;
    let room = general(&server).await;
    upload_ok(&server, &alice, room, &[1; 3000]).await;
    upload_ok(&server, &alice, room, &[1; 4000]).await;
    for s in [&moder, &alice] {
        let (status, _, _) = server.call(get("/api/admin/disk", s)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let (status, _, v) = server.call(get("/api/admin/disk", &owner)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let d: vesper_protocol::DiskUse = serde_json::from_value(v).unwrap();
    assert_eq!(d.upload_bytes, 7000);
    assert_eq!(d.quota_bytes, 10 * 1024);
    assert!(d.db_bytes > 0);
    assert!(d.fs_total > 0 && (0..=d.fs_total).contains(&d.fs_free));
    // 7000 of 10240 is under 80%, so only a full filesystem warns.
    let fs_full = (d.fs_total - d.fs_free) * 5 > d.fs_total * 4;
    assert_eq!(d.warn, fs_full);
    upload_ok(&server, &alice, room, &[1; 1500]).await;
    let (_, _, v) = server.call(get("/api/admin/disk", &owner)).await;
    assert_eq!(v["warn"], true);
}

/// Admin > Server: owner only; the build, the disk numbers of Admin > Disk use, who is in voice
/// and the newest backup in `<data>/backups`. No addresses and no secrets in the answer.
#[tokio::test]
async fn server_status_is_for_the_owner() {
    let server = TestServer::new().await;
    let owner = server.account("olive", Role::Owner).await;
    let moder = server.account("mo", Role::Moderator).await;
    let member = server.account("mel", Role::Member).await;
    for who in [&moder, &member] {
        let (status, _, _) = server.call(get("/api/admin/server", who)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let (status, _, v) = server.call(get("/api/admin/server", &owner)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let s: vesper_protocol::ServerStatus = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(s.server.build, "test-build");
    assert!(s.disk.db_bytes > 0 && s.disk.fs_total > 0);
    assert_eq!((s.voice, s.push_failures, s.last_backup), (0, 0, None));
    let text = v.to_string();
    for no in ["127.0.0.1", "session", "token", "vapid", "endpoint"] {
        assert!(!text.to_ascii_lowercase().contains(no), "{no} in {text}");
    }
    let dir = server.data.path().join("backups");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("old.db"), b"x").unwrap();
    std::fs::write(dir.join("old.json"), b"{}").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(dir.join("pre-0009-to-0010-20261102T000000Z.db"), b"x").unwrap();
    let (_, _, v) = server.call(get("/api/admin/server", &owner)).await;
    assert_eq!(
        v["last_backup"]["name"],
        "pre-0009-to-0010-20261102T000000Z.db"
    );
    assert!(v["last_backup"]["at"].as_i64().unwrap() > 1_700_000_000_000);
}

/// Raw HTTP/1.1 GET to the whole app (with `guard`), the path sent byte for byte.
async fn wire_get(server: &TestServer, path: &str, s: &Session) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut t = tokio::net::TcpStream::connect(server.addr).await.unwrap();
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: x\r\nCookie: {}\r\nConnection: close\r\n\r\n",
        s.cookie
    );
    t.write_all(req.as_bytes()).await.unwrap();
    let mut b = Vec::new();
    t.read_to_end(&mut b).await.unwrap();
    String::from_utf8_lossy(&b).to_ascii_lowercase()
}

#[tokio::test]
async fn served_over_http_keeps_upload_headers() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let id = upload_ok(&server, &alice, general(&server).await, b"<html>").await;
    let res = wire_get(&server, &format!("/api/uploads/{id}"), &alice).await;
    assert!(res.starts_with("http/1.1 200"), "{res}");
    for want in [
        "cache-control: private, max-age=31536000, immutable",
        "content-security-policy: default-src 'none'; sandbox",
        "content-type: application/octet-stream",
        "x-content-type-options: nosniff",
        "cross-origin-resource-policy: same-origin",
    ] {
        assert!(res.contains(want), "{want}: {res}");
    }
    for bad in [
        "/api/uploads/../../vesper.db",
        "/api/uploads/..%2f..%2fvesper.db",
        "/api/uploads/%2e%2e/vesper.db",
        "/api/uploads/%2fetc%2fpasswd",
    ] {
        let res = wire_get(&server, bad, &alice).await;
        assert!(res.starts_with("http/1.1 404"), "{bad}: {res}");
        assert!(!res.contains("sqlite"), "{bad}");
    }
}

#[tokio::test]
async fn upload_keeps_sandbox_csp() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let id = upload_ok(&server, &alice, general(&server).await, &png()).await;
    let res = wire_get(&server, &format!("/api/uploads/{id}"), &alice).await;
    assert!(res.starts_with("http/1.1 200"), "{res}");
    // Exactly one policy, the sandbox: the page's global CSP must not replace or join it.
    let csp: Vec<&str> = res
        .lines()
        .filter(|l| l.starts_with("content-security-policy:"))
        .collect();
    assert_eq!(
        csp,
        ["content-security-policy: default-src 'none'; sandbox"],
        "{res}"
    );
    // The other security headers still come from `guard`.
    for want in [
        "referrer-policy: no-referrer",
        "x-content-type-options: nosniff",
        "cross-origin-opener-policy: same-origin",
    ] {
        assert!(res.contains(want), "{want}: {res}");
    }
}

/// A declared Content-Length that cannot fit is 507 before any byte is read: the body here
/// never ends, so reading it would hang the test.
#[tokio::test]
async fn early_quota_check_skips_the_body() {
    let server = quota_server().await;
    let alice = server.account("alice", Role::Member).await;
    let room = general(&server).await;
    upload_ok(&server, &alice, room, &vec![1; 600 * KIB]).await;
    let never = Body::from_stream(futures_util::stream::pending::<Result<Bytes, std::io::Error>>());
    let mut req = upload_req(room, "big", never, &alice);
    req.headers_mut().insert(
        header::CONTENT_LENGTH,
        (600 * KIB).to_string().parse().unwrap(),
    );
    let (status, _, v) = tokio::time::timeout(std::time::Duration::from_secs(5), server.call(req))
        .await
        .expect("the body was read");
    assert_eq!(status, StatusCode::INSUFFICIENT_STORAGE, "{v}");
    assert_eq!(v["code"], "quota_full");
    assert!(entries(&tmp_dir(&server)).is_empty());
    // A declared size that fits still goes through.
    let mut req = upload_req(room, "ok", Body::from(vec![1u8; 100 * KIB]), &alice);
    req.headers_mut().insert(
        header::CONTENT_LENGTH,
        (100 * KIB).to_string().parse().unwrap(),
    );
    assert_eq!(server.call(req).await.0, StatusCode::CREATED);
}

#[tokio::test]
async fn orphan_files_are_swept() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let room = general(&server).await;
    let kept = upload_ok(&server, &alice, room, b"kept").await;
    // A file with no row (a failed delete, or a crash before the row insert).
    let orphan = "ab".repeat(16);
    let path = file_of(&server, &orphan);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"orphan").unwrap();
    // Names this code never writes are left alone.
    let odd = path.with_file_name("notes.txt");
    std::fs::write(&odd, b"x").unwrap();
    let day = std::time::Duration::from_secs(24 * 3600);
    let now = std::time::SystemTime::now();
    // Too new: kept.
    assert_eq!(uploads::sweep_orphans(&server.state, now).await.unwrap(), 0);
    assert!(path.exists());
    // A dir the sweep cannot read is logged and skipped; the rest of the pass still runs.
    use std::os::unix::fs::PermissionsExt;
    let locked = file_of(&server, &"cd".repeat(16));
    let locked = locked.parent().unwrap();
    std::fs::create_dir_all(locked).unwrap();
    std::fs::set_permissions(locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let later = now + day + std::time::Duration::from_secs(60);
    let swept = uploads::sweep_orphans(&server.state, later).await;
    std::fs::set_permissions(locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(swept.unwrap(), 1);
    assert!(!path.exists() && odd.exists());
    assert!(file_of(&server, &kept).exists(), "a file with a row stays");
    assert_eq!(download(&server, &kept, &alice).await.0, StatusCode::OK);
}
