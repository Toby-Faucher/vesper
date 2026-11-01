//! Profile pictures and bios over HTTP: what is refused, who can change what, and the headers a
//! picture is served with.

mod common;

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use common::{ORIGIN, Session, TestServer, get, request};
use serde_json::{Value, json};

fn png(extra: &[u8]) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [
        (&b"IHDR"[..], &[1u8; 13][..]),
        (b"IDAT", extra),
        (b"IEND", b""),
    ] {
        b.extend((data.len() as u32).to_be_bytes());
        b.extend(kind);
        b.extend(data);
        b.extend([0; 4]);
    }
    b
}

fn put_avatar(bytes: Vec<u8>, content_type: &str, s: &Session) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri("/api/me/avatar")
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, &s.cookie)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(bytes))
        .unwrap()
}

async fn put(t: &TestServer, bytes: Vec<u8>, s: &Session) -> (StatusCode, Value) {
    let (st, _, v) = t.call(put_avatar(bytes, "image/png", s)).await;
    (st, v)
}

async fn fetch(t: &TestServer, path: &str, s: &Session) -> (StatusCode, HeaderMap, Value) {
    t.call(get(path, s)).await
}

#[tokio::test]
async fn upload_serve_replace_and_remove() {
    let t = TestServer::new().await;
    let alice = t.member("alice").await;
    let bob = t.member("bob").await;
    let id = &alice.user.user_id;
    assert_eq!(alice.user.avatar, None);
    let url = format!("/api/users/{id}/avatar");
    assert_eq!(fetch(&t, &url, &bob).await.0, StatusCode::NOT_FOUND);

    let (s, v) = put(&t, png(b"one"), &alice).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let ver = v["avatar"].as_str().unwrap().to_string();
    assert_eq!(ver.len(), 16);
    // Anyone signed in may see it, with the uploads' headers.
    let (s, h, _) = fetch(&t, &format!("{url}?v={ver}"), &bob).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h[header::CONTENT_TYPE], "image/png");
    assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(
        h[header::CONTENT_SECURITY_POLICY],
        "default-src 'none'; sandbox"
    );
    assert_eq!(h["cross-origin-resource-policy"], "same-origin");
    assert_eq!(h[header::ETAG], format!("\"{ver}\"").as_str());
    assert!(
        h[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    // Without the version it must be revalidated; with the ETag that is a 304.
    let (_, h, _) = fetch(&t, &url, &bob).await;
    assert_eq!(h[header::CACHE_CONTROL], "private, no-cache");
    let mut req = get(&url, &bob);
    req.headers_mut()
        .insert(header::IF_NONE_MATCH, format!("\"{ver}\"").parse().unwrap());
    assert_eq!(t.call(req).await.0, StatusCode::NOT_MODIFIED);
    // Signed out: no picture.
    let (s, _, _) = t.call(request(Method::GET, &url, None, None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // A new picture is a new version; the user list shows it.
    let (_, v) = put(&t, png(b"two"), &alice).await;
    assert_ne!(v["avatar"], ver.as_str());
    let (_, _, users) = fetch(&t, "/api/users", &bob).await;
    let me = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["user_id"] == id.as_str())
        .unwrap();
    assert_eq!(me["avatar"], v["avatar"]);

    let del = request(Method::DELETE, "/api/me/avatar", None, Some(&alice.cookie));
    let (s, _, v) = t.call(del).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["avatar"], Value::Null);
    assert_eq!(fetch(&t, &url, &bob).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn non_images_svg_polyglots_and_oversize_are_refused() {
    let t = TestServer::new().await;
    let alice = t.member("alice").await;
    let mut after_iend = png(b"x");
    after_iend.extend(b"<html><script>alert(1)</script>");
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>".to_vec();
    for (bytes, why) in [
        (b"just text".to_vec(), "text"),
        (svg.clone(), "svg"),
        (b"GIF89a\x01\0\x01\0\0\0\0;".to_vec(), "gif"),
        (after_iend, "html after the png"),
        (png(b"<script>alert(1)</script>"), "script inside the png"),
        (Vec::new(), "empty"),
    ] {
        let (s, v) = put(&t, bytes, &alice).await;
        assert_eq!(
            (s, v["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("bad_request")),
            "{why}"
        );
    }
    // The client's content type changes nothing.
    let (s, _, _) = t.call(put_avatar(svg, "image/png", &alice)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // 512 KiB is the cap.
    let (s, v) = put(&t, png(&vec![7; 512 * 1024]), &alice).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::PAYLOAD_TOO_LARGE, Some("too_large"))
    );
    let (s, v) = put(&t, png(&vec![7; 512 * 1024 - 100]), &alice).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn nobody_changes_someone_elses_picture() {
    let t = TestServer::new().await;
    let alice = t.member("alice").await;
    let owner = t.owner().await;
    let (_, v) = put(&t, png(b"a"), &alice).await;
    let ver = v["avatar"].clone();
    // There is no route for another user's picture, even for the owner.
    let other = format!("/api/users/{}/avatar", alice.user.user_id);
    for m in [Method::PUT, Method::DELETE, Method::POST, Method::PATCH] {
        let req = Request::builder()
            .method(m.clone())
            .uri(&other)
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, &owner.cookie)
            .body(Body::from(png(b"b")))
            .unwrap();
        assert_eq!(t.call(req).await.0, StatusCode::METHOD_NOT_ALLOWED, "{m}");
    }
    // The owner's own upload changes only the owner.
    put(&t, png(b"b"), &owner).await;
    let (_, _, me) = fetch(&t, "/api/me", &alice).await;
    assert_eq!(me["user"]["avatar"], ver);
}

#[tokio::test]
async fn upload_is_rate_limited() {
    let t = TestServer::new().await;
    let alice = t.member("alice").await;
    for i in 0..10 {
        assert_eq!(put(&t, png(&[i]), &alice).await.0, StatusCode::OK);
    }
    let (s, v) = put(&t, png(b"x"), &alice).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited"))
    );
}

#[tokio::test]
async fn bio_is_checked_like_a_display_name() {
    let t = TestServer::new().await;
    let a = t.member("alice").await;
    let patch = |v: Value| request(Method::PATCH, "/api/me", Some(&v), Some(&a.cookie));
    let (s, _, v) = t.call(patch(json!({ "bio": "  plays  bass\n" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["bio"], "plays bass");
    for bad in [
        "é".repeat(161),
        "hi\u{202e}there".into(),
        "a\u{200b}b".into(),
    ] {
        let (s, _, v) = t.call(patch(json!({ "bio": bad }))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad:?} {v}");
    }
    let (s, _, v) = t.call(patch(json!({ "bio": "é".repeat(160) }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // Leaving it out keeps it; empty clears it.
    let (_, _, v) = t.call(patch(json!({ "display_name": "Al" }))).await;
    assert_eq!(v["bio"], "é".repeat(160));
    let (_, _, v) = t.call(patch(json!({ "bio": "" }))).await;
    assert_eq!(v["bio"], "");
}
