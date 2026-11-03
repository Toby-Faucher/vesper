//! Web Push end to end against a fake push service on 127.0.0.1 (the `test-push` feature lets
//! the server reach it; release builds refuse that feature).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};
use aws_lc_rs::{aead, agreement, hkdf};
use axum::http::{Method, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use common::{Session, TestServer, get, post, request};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use vesper_protocol::{PushPayload, PushPayloadKind, Role, RoomInfo};

const SUBJECT: &str = "mailto:test@example.invalid";

struct Got {
    head: String,
    body: Vec<u8>,
}

/// A push service: records every request and answers `status`.
struct Fake {
    port: u16,
    got: mpsc::UnboundedReceiver<Got>,
    status: Arc<AtomicU16>,
}

impl Fake {
    async fn start() -> Fake {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, got) = mpsc::unbounded_channel();
        let status = Arc::new(AtomicU16::new(201));
        let code = status.clone();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                let (tx, code) = (tx.clone(), code.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let end = loop {
                        let n = s.read(&mut chunk).await.unwrap();
                        assert!(n > 0, "request ended early");
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                    };
                    let head = String::from_utf8(buf[..end].to_vec()).unwrap();
                    let len: usize = header(&head, "content-length").unwrap().parse().unwrap();
                    while buf.len() < end + len {
                        let n = s.read(&mut chunk).await.unwrap();
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let c = code.load(Ordering::SeqCst);
                    let _ = tx.send(Got {
                        head,
                        body: buf[end..end + len].to_vec(),
                    });
                    if c == 0 {
                        // A push service that never answers: hold the connection open.
                        tokio::time::sleep(Duration::from_secs(3600)).await;
                    }
                    let reply = format!("HTTP/1.1 {c} X\r\nContent-Length: 0\r\n\r\n");
                    let _ = s.write_all(reply.as_bytes()).await;
                });
            }
        });
        Fake { port, got, status }
    }

    async fn next(&mut self) -> Got {
        tokio::time::timeout(Duration::from_secs(5), self.got.recv())
            .await
            .expect("a push in time")
            .unwrap()
    }

    /// Nothing arrives for a while.
    async fn quiet(&mut self) {
        let r = tokio::time::timeout(Duration::from_millis(500), self.got.recv()).await;
        assert!(
            r.is_err(),
            "unexpected push: {:?}",
            r.unwrap().map(|g| g.head)
        );
    }
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

/// A browser's push keys: the private half stays here to decrypt.
struct Browser {
    key: agreement::PrivateKey,
    auth: [u8; 16],
}

impl Browser {
    fn new() -> Browser {
        Browser {
            key: agreement::PrivateKey::generate(&agreement::ECDH_P256).unwrap(),
            auth: [7; 16],
        }
    }

    fn subscription(&self, endpoint: &str, mentions: bool, voice: bool) -> Value {
        let public = self.key.compute_public_key().unwrap();
        json!({
            "endpoint": endpoint,
            "p256dh": B64.encode(public.as_ref()),
            "auth": B64.encode(self.auth),
            "mentions": mentions,
            "voice": voice,
        })
    }

    /// RFC 8291 decryption, written apart from the server's code.
    fn decrypt(&self, body: &[u8]) -> Vec<u8> {
        struct Len(usize);
        impl hkdf::KeyType for Len {
            fn len(&self) -> usize {
                self.0
            }
        }
        let expand = |prk: &hkdf::Prk, info: &[u8], n: usize| {
            let mut out = vec![0; n];
            prk.expand(&[info], Len(n)).unwrap().fill(&mut out).unwrap();
            out
        };
        let (salt, rs, idlen) = (&body[..16], &body[16..20], body[20]);
        assert_eq!((rs, idlen), (&4096u32.to_be_bytes()[..], 65));
        let as_public = &body[21..86];
        let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, as_public);
        let ecdh = agreement::agree(&self.key, peer, (), |s| Ok(s.to_vec())).unwrap();
        let ua_public = self.key.compute_public_key().unwrap();
        let prk_key = hkdf::Salt::new(hkdf::HKDF_SHA256, &self.auth).extract(&ecdh);
        let info = [b"WebPush: info\0".as_slice(), ua_public.as_ref(), as_public].concat();
        let ikm = expand(&prk_key, &info, 32);
        let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(&ikm);
        let cek = expand(&prk, b"Content-Encoding: aes128gcm\0", 16);
        let nonce = expand(&prk, b"Content-Encoding: nonce\0", 12);
        let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &cek).unwrap());
        let mut data = body[86..].to_vec();
        let plain = key
            .open_in_place(
                aead::Nonce::try_assume_unique_for_key(&nonce).unwrap(),
                aead::Aad::empty(),
                &mut data,
            )
            .unwrap();
        assert_eq!(plain.last(), Some(&2), "last-record delimiter");
        plain[..plain.len() - 1].to_vec()
    }
}

fn put(path: &str, v: Value, s: &Session) -> axum::http::Request<axum::body::Body> {
    request(Method::PUT, path, Some(&v), Some(&s.cookie))
}

async fn room(server: &TestServer, s: &Session, name: &str) -> RoomInfo {
    let (_, _, v) = server.call(get("/api/rooms", s)).await;
    let rooms: Vec<RoomInfo> = serde_json::from_value(v).unwrap();
    rooms.into_iter().find(|r| r.name == name).unwrap()
}

/// Subscribes with a full rate-limit bucket (see `subscribe_is_rate_limited`).
async fn subscribe(server: &TestServer, s: &Session, sub: Value) -> (StatusCode, Value) {
    server.state.limits.advance(Duration::from_secs(3600));
    let (st, _, v) = server.call(put("/api/push/subscription", sub, s)).await;
    (st, v)
}

async fn say(server: &TestServer, s: &Session, room: i64, body: &str) {
    let path = format!("/api/rooms/{room}/messages");
    let (st, _, v) = server.call(post(&path, json!({ "body": body }), s)).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
}

#[tokio::test]
async fn a_mention_reaches_the_push_service_signed_and_encrypted() {
    let server = TestServer::new().await;
    let mut fake = Fake::start().await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let general = room(&server, &alice, "general").await;
    let phone = Browser::new();
    let endpoint = format!("http://127.0.0.1:{}/push/bob-1", fake.port);
    let (st, v) = subscribe(&server, &bob, phone.subscription(&endpoint, true, false)).await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{v}");

    say(
        &server,
        &alice,
        general.id,
        "hey @bob the secret plan is ready",
    )
    .await;
    let got = fake.next().await;
    assert!(
        got.head.starts_with("POST /push/bob-1 HTTP/1.1\r\n"),
        "{}",
        got.head
    );
    assert_eq!(header(&got.head, "content-encoding"), Some("aes128gcm"));
    assert_eq!(header(&got.head, "ttl"), Some("86400"));
    assert_eq!(header(&got.head, "urgency"), Some("high"));
    assert!(header(&got.head, "cookie").is_none());

    // VAPID: the JWT verifies with the key the client was given, for this audience.
    let (_, _, info) = server.call(get("/api/push", &bob)).await;
    let k = info["public_key"].as_str().unwrap();
    let auth = header(&got.head, "authorization").unwrap();
    let (jwt, key) = auth
        .strip_prefix("vapid t=")
        .unwrap()
        .split_once(", k=")
        .unwrap();
    assert_eq!(key, k);
    let (signed, sig) = jwt.rsplit_once('.').unwrap();
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, B64.decode(k).unwrap())
        .verify(signed.as_bytes(), &B64.decode(sig).unwrap())
        .expect("VAPID signature");
    let claims: Value =
        serde_json::from_slice(&B64.decode(signed.split('.').nth(1).unwrap()).unwrap()).unwrap();
    assert_eq!(claims["aud"], format!("http://127.0.0.1:{}", fake.port));
    assert_eq!(claims["sub"], SUBJECT);

    // The push service sees ciphertext only; the browser sees names, never the message.
    for clear in ["secret", "alice", "general", "bob"] {
        assert!(
            !String::from_utf8_lossy(&got.body).contains(clear),
            "{clear}"
        );
    }
    let p: PushPayload = serde_json::from_slice(&phone.decrypt(&got.body)).unwrap();
    assert_eq!(p.kind, PushPayloadKind::Mention);
    assert_eq!(p.room_id, general.id);
    assert_eq!(p.title, "Vesper");
    assert_eq!(p.body, "alice mentioned you in general");

    // The device list shows the delivery; the API never echoes the endpoint.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (_, _, info) = server.call(get("/api/push", &bob)).await;
    assert!(info["devices"][0]["last_ok_at"].is_i64(), "{info}");
    assert!(info["devices"][0]["current"].as_bool().unwrap());
    assert!(!info.to_string().contains("bob-1"));
}

#[tokio::test]
async fn no_push_with_a_socket_open_to_the_sender_when_hidden_or_switched_off() {
    let server = TestServer::new().await;
    let mut fake = Fake::start().await;
    let owner = server.account("owner", Role::Owner).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let carol = server.account("carol", Role::Member).await;
    let general = room(&server, &alice, "general").await;
    for (s, name, mentions) in [
        (&bob, "bob", true),
        (&alice, "alice", true),
        (&carol, "carol", false),
    ] {
        let endpoint = format!("http://127.0.0.1:{}/push/{name}", fake.port);
        let (st, v) = subscribe(
            &server,
            s,
            Browser::new().subscription(&endpoint, mentions, true),
        )
        .await;
        assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    }
    // bob has a Vesper tab open: the tab notifies, so no push.
    let (tx, _rx) = vesper_server::hub::outbox();
    let conn = server.state.hub.add(bob.user.clone(), 0, tx);
    say(&server, &alice, general.id, "@bob are you there").await;
    fake.quiet().await;
    server.state.hub.remove(conn);
    // Mentioning yourself, and carol with mentions off on her only device.
    say(
        &server,
        &alice,
        general.id,
        "@alice note to self, @carol fyi",
    )
    .await;
    fake.quiet().await;
    // A restricted room bob is not in: the mention does not resolve to him.
    let (st, _, v) = server
        .call(post("/api/rooms", json!({ "name": "staff" }), &owner))
        .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let staff: RoomInfo = serde_json::from_value(v).unwrap();
    let (st, _, _) = server
        .call(request(
            Method::PATCH,
            &format!("/api/rooms/{}", staff.id),
            Some(&json!({ "restricted": true })),
            Some(&owner.cookie),
        ))
        .await;
    assert_eq!(st, StatusCode::OK);
    say(&server, &owner, staff.id, "@bob secret staff talk").await;
    fake.quiet().await;
    // And then the same mention in a room bob sees does arrive.
    say(&server, &alice, general.id, "@bob now").await;
    assert!(fake.next().await.head.starts_with("POST /push/bob "));
}

#[tokio::test]
async fn dm_push_follows_the_dms_switch_not_the_mentions_one() {
    let server = TestServer::new().await;
    let mut fake = Fake::start().await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let general = room(&server, &alice, "general").await;
    // bob has mentions on and direct messages off.
    let endpoint = format!("http://127.0.0.1:{}/push/bob", fake.port);
    let mut sub = Browser::new().subscription(&endpoint, true, false);
    sub["dms"] = json!(false);
    let (st, v) = subscribe(&server, &bob, sub).await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    let (st, _, v) = server
        .call(post(
            "/api/dms",
            json!({ "user_id": bob.user.user_id }),
            &alice,
        ))
        .await;
    assert!(st.is_success(), "{v}");
    say(&server, &alice, v["id"].as_i64().unwrap(), "hello").await;
    fake.quiet().await;
    // The mentions switch still works.
    say(&server, &alice, general.id, "@bob hi").await;
    assert!(fake.next().await.head.starts_with("POST /push/bob "));
    // The device list shows the switch.
    let (_, _, v) = server.call(get("/api/push", &bob)).await;
    assert_eq!(v["devices"][0]["dms"], json!(false));
}

#[tokio::test]
async fn voice_started_goes_once_per_ten_minutes_to_who_can_see_the_room() {
    let server = TestServer::new().await;
    let mut fake = Fake::start().await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let carol = server.account("carol", Role::Member).await;
    let general = room(&server, &alice, "general").await;
    let phone = Browser::new();
    let endpoint = format!("http://127.0.0.1:{}/push/bob", fake.port);
    subscribe(&server, &bob, phone.subscription(&endpoint, false, true)).await;
    let endpoint = format!("http://127.0.0.1:{}/push/carol", fake.port);
    subscribe(
        &server,
        &carol,
        Browser::new().subscription(&endpoint, true, false),
    )
    .await;

    server.state.push.voice_started(&general, &alice.user).await;
    let got = fake.next().await;
    assert!(
        got.head.starts_with("POST /push/bob "),
        "carol has voice off"
    );
    assert_eq!(header(&got.head, "ttl"), Some("600"));
    let p: PushPayload = serde_json::from_slice(&phone.decrypt(&got.body)).unwrap();
    assert_eq!(
        (p.kind, p.body.as_str()),
        (PushPayloadKind::Voice, "alice started voice in general")
    );
    fake.quiet().await;
    // Again within ten minutes: nothing.
    server.state.push.voice_started(&general, &alice.user).await;
    fake.quiet().await;
}

#[tokio::test]
async fn only_the_first_join_starts_voice() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let general = room(&server, &alice, "general").await;
    let mut keep = vec![];
    for (s, first) in [(&alice, true), (&bob, false)] {
        let (tx, rx) = vesper_server::hub::outbox();
        let client = vesper_sfu::testing::FakeClient::new().unwrap();
        let j = server
            .state
            .voice
            .join(&s.user, &general, client.offer().into(), tx)
            .await
            .unwrap();
        assert_eq!(j.first, first, "{}", s.user.username);
        keep.push((rx, client));
    }
}

#[tokio::test]
async fn a_gone_subscription_is_deleted() {
    let server = TestServer::new().await;
    let fake = Fake::start().await;
    let bob = server.account("bob", Role::Member).await;
    let owner = server.account("olive", Role::Owner).await;
    let failures =
        async || server.call(get("/api/admin/server", &owner)).await.2["push_failures"].clone();
    let endpoint = format!("http://127.0.0.1:{}/push/bob", fake.port);
    subscribe(
        &server,
        &bob,
        Browser::new().subscription(&endpoint, true, true),
    )
    .await;
    let (st, _, v) = server.call(post("/api/push/test", json!({}), &bob)).await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    fake.status.store(410, Ordering::SeqCst);
    let (st, _, _) = server.call(post("/api/push/test", json!({}), &bob)).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (_, _, info) = server.call(get("/api/push", &bob)).await;
    assert_eq!(info["devices"], json!([]));
    assert_eq!(failures().await, 0, "a gone subscription is not a failure");
    // A refusal that is not 404 or 410 keeps the subscription.
    subscribe(
        &server,
        &bob,
        Browser::new().subscription(&endpoint, true, true),
    )
    .await;
    fake.status.store(500, Ordering::SeqCst);
    let (st, _, _) = server.call(post("/api/push/test", json!({}), &bob)).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR);
    let (_, _, info) = server.call(get("/api/push", &bob)).await;
    assert_eq!(info["devices"].as_array().unwrap().len(), 1);
    assert_eq!(
        failures().await,
        1,
        "Admin > Server counts the refused push"
    );
}

#[tokio::test]
async fn subscriptions_are_checked_capped_and_removed() {
    let server = TestServer::new().await;
    let bob = server.account("bob", Role::Member).await;
    let alice = server.account("alice", Role::Member).await;
    let b = Browser::new();
    for bad in [
        "https://10.0.0.1/x",
        "https://169.254.169.254/latest",
        "http://fcm.googleapis.com/fcm/send/x",
        "https://localhost/x",
        "https://user@fcm.googleapis.com/x",
        "https://evil.example/x",
        "http://localhost:9/x",
        "http://127.0.0.2:9/x",
        "http://10.0.0.1:9/x",
    ] {
        let (st, _) = subscribe(&server, &bob, b.subscription(bad, true, true)).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{bad}");
    }
    let ok = "https://fcm.googleapis.com/fcm/send/x";
    let mut keys = b.subscription(ok, true, true);
    keys["p256dh"] = json!(B64.encode([4u8; 65])); // not a point on the curve
    assert_eq!(
        subscribe(&server, &bob, keys).await.0,
        StatusCode::BAD_REQUEST
    );
    let mut keys = b.subscription(ok, true, true);
    keys["auth"] = json!("short");
    assert_eq!(
        subscribe(&server, &bob, keys).await.0,
        StatusCode::BAD_REQUEST
    );
    // Without our Origin, or signed out.
    let req = request(
        Method::PUT,
        "/api/push/subscription",
        Some(&b.subscription(ok, true, true)),
        None,
    );
    assert_eq!(server.call(req).await.0, StatusCode::UNAUTHORIZED);
    let mut req = put(
        "/api/push/subscription",
        b.subscription(ok, true, true),
        &bob,
    );
    req.headers_mut().remove("origin");
    assert_eq!(server.call(req).await.0, StatusCode::FORBIDDEN);

    // Five devices, then a sixth is refused.
    for i in 0..5 {
        let s = server.session(&bob.user).await;
        let e = format!("https://fcm.googleapis.com/fcm/send/{i}");
        assert_eq!(
            subscribe(&server, &s, b.subscription(&e, true, true))
                .await
                .0,
            StatusCode::NO_CONTENT
        );
    }
    let (st, v) = subscribe(&server, &bob, b.subscription(ok, true, true)).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    let (_, _, info) = server.call(get("/api/push", &bob)).await;
    let id = info["devices"][0]["id"].as_i64().unwrap();
    // Someone else cannot remove bob's device; bob can.
    let del = |s: &Session| {
        request(
            Method::DELETE,
            &format!("/api/push/devices/{id}"),
            None,
            Some(&s.cookie),
        )
    };
    assert_eq!(server.call(del(&alice)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(server.call(del(&bob)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(
        subscribe(&server, &bob, b.subscription(ok, true, true))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    // Turning push off on this device, and logging out, drop this device's subscription.
    let off = request(
        Method::DELETE,
        "/api/push/subscription",
        None,
        Some(&bob.cookie),
    );
    assert_eq!(server.call(off).await.0, StatusCode::NO_CONTENT);
    let (_, _, info) = server.call(get("/api/push", &bob)).await;
    assert_eq!(info["devices"].as_array().unwrap().len(), 4);
    subscribe(&server, &bob, b.subscription(ok, true, true)).await;
    let (st, _, _) = server.call(post("/api/logout", json!({}), &bob)).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let other = server.session(&bob.user).await;
    let (_, _, info) = server.call(get("/api/push", &other)).await;
    assert_eq!(info["devices"].as_array().unwrap().len(), 4);
    assert!(!info.to_string().contains("fcm"));
}

#[tokio::test]
async fn subscribe_is_rate_limited() {
    let server = TestServer::new().await;
    let bob = server.account("bob", Role::Member).await;
    let b = Browser::new();
    let ok = "https://fcm.googleapis.com/fcm/send/x";
    let once = || {
        put(
            "/api/push/subscription",
            b.subscription(ok, true, true),
            &bob,
        )
    };
    for _ in 0..20 {
        assert_eq!(server.call(once()).await.0, StatusCode::NO_CONTENT);
    }
    assert_eq!(server.call(once()).await.0, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn push_is_off_without_a_subject() {
    let server = TestServer::with(|c| c.push_subject = None, common::spawn_sfu()).await;
    let bob = server.account("bob", Role::Member).await;
    let (_, _, info) = server.call(get("/api/push", &bob)).await;
    assert_eq!(info["public_key"], Value::Null);
    let sub = Browser::new().subscription("https://fcm.googleapis.com/fcm/send/x", true, true);
    assert_eq!(subscribe(&server, &bob, sub).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_rejected_vapid_key_deletes_the_subscription() {
    let server = TestServer::new().await;
    let fake = Fake::start().await;
    let bob = server.account("bob", Role::Member).await;
    let endpoint = format!("http://127.0.0.1:{}/push/bob", fake.port);
    for code in [401, 403] {
        subscribe(
            &server,
            &bob,
            Browser::new().subscription(&endpoint, true, true),
        )
        .await;
        fake.status.store(code, Ordering::SeqCst);
        let (st, _, _) = server.call(post("/api/push/test", json!({}), &bob)).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{code}");
        let (_, _, info) = server.call(get("/api/push", &bob)).await;
        assert_eq!(info["devices"], json!([]), "{code}");
    }
}

#[tokio::test]
async fn another_account_cannot_take_an_endpoint_without_its_keys() {
    let server = TestServer::new().await;
    let bob = server.account("bob", Role::Member).await;
    let eve = server.account("eve", Role::Member).await;
    let e = "https://fcm.googleapis.com/fcm/send/bob";
    let b = Browser::new();
    assert_eq!(
        subscribe(&server, &bob, b.subscription(e, true, true))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let (st, v) = subscribe(&server, &eve, Browser::new().subscription(e, true, true)).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    let (_, _, info) = server.call(get("/api/push", &bob)).await;
    assert_eq!(info["devices"].as_array().unwrap().len(), 1);
    // The same browser (same keys) signed in as eve does move.
    assert_eq!(
        subscribe(&server, &eve, b.subscription(e, true, true))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let (_, _, info) = server.call(get("/api/push", &bob)).await;
    assert_eq!(info["devices"], json!([]));
}

/// A server whose push sender has a short timeout and a small queue.
async fn limited(timeout: Duration, max_pending: usize) -> TestServer {
    let server = TestServer::new().await;
    let push = vesper_server::push::Push::with_limits(
        server.state.store.clone(),
        server.state.hub.clone(),
        Some(SUBJECT.into()),
        timeout,
        max_pending,
    )
    .await
    .unwrap();
    TestServer {
        state: vesper_server::AppState {
            push,
            ..server.state.clone()
        },
        ..server
    }
}

#[tokio::test]
async fn a_push_service_that_never_answers_frees_its_slot_in_time() {
    let server = limited(Duration::from_millis(300), 256).await;
    let mut fake = Fake::start().await;
    fake.status.store(0, Ordering::SeqCst);
    let bob = server.account("bob", Role::Member).await;
    let endpoint = format!("http://127.0.0.1:{}/push/bob", fake.port);
    subscribe(
        &server,
        &bob,
        Browser::new().subscription(&endpoint, true, true),
    )
    .await;
    let t = std::time::Instant::now();
    let (st, _, _) = server.call(post("/api/push/test", json!({}), &bob)).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    fake.next().await;
    // A failure that is not a refusal keeps the subscription.
    let (_, _, info) = server.call(get("/api/push", &bob)).await;
    assert_eq!(info["devices"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn sends_past_the_queue_cap_are_dropped() {
    let server = limited(Duration::from_secs(5), 2).await;
    let mut fake = Fake::start().await;
    fake.status.store(0, Ordering::SeqCst);
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let general = room(&server, &alice, "general").await;
    for i in 0..4 {
        let s = server.session(&bob.user).await;
        let e = format!("http://127.0.0.1:{}/push/{i}", fake.port);
        subscribe(&server, &s, Browser::new().subscription(&e, true, true)).await;
    }
    server
        .state
        .push
        .mentioned(
            &general,
            &alice.user,
            std::slice::from_ref(&bob.user.user_id),
        )
        .await;
    fake.next().await;
    fake.next().await;
    fake.quiet().await;
    assert_eq!(server.state.push.pending(), 2);
    // Once the stuck ones time out, the queue is empty again.
    for _ in 0..40 {
        if server.state.push.pending() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(server.state.push.pending(), 0);
}
