//! Passkeys, driven by a software authenticator built here from aws-lc-rs: P-256 keys, "none"
//! attestation, hand-made CBOR. Not a real authenticator and not a real browser.

mod common;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};
use axum::http::{Method, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use common::{ORIGIN, Session, TestServer, get, post, request};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const RP_ID: &str = "localhost";

struct Soft {
    key: EcdsaKeyPair,
    id: Vec<u8>,
    counter: u32,
}

fn cbor_bytes(out: &mut Vec<u8>, b: &[u8]) {
    assert!((24..256).contains(&b.len()));
    out.extend([0x58, b.len() as u8]);
    out.extend(b);
}

impl Soft {
    fn new() -> Soft {
        let pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &SystemRandom::new())
                .unwrap();
        let key =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref()).unwrap();
        Soft {
            key,
            id: (0..32).map(|i| i * 7 + 1).collect(),
            counter: 0,
        }
    }

    fn client_data(kind: &str, options: &Value, origin: &str) -> Vec<u8> {
        let challenge = options["publicKey"]["challenge"].as_str().unwrap();
        json!({ "type": kind, "challenge": challenge, "origin": origin, "crossOrigin": false })
            .to_string()
            .into_bytes()
    }

    fn rp_hash() -> [u8; 32] {
        Sha256::digest(RP_ID.as_bytes()).into()
    }

    /// The answer to `navigator.credentials.create`, signed as if the page was at `origin`.
    fn create(&self, options: &Value, origin: &str) -> Value {
        let point = self.key.public_key().as_ref(); // 0x04 || x || y
        let mut cose = vec![0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21];
        cbor_bytes(&mut cose, &point[1..33]);
        cose.push(0x22);
        cbor_bytes(&mut cose, &point[33..65]);
        let mut auth = Self::rp_hash().to_vec();
        auth.push(0x01 | 0x04 | 0x40); // user present, user verified, attested data
        auth.extend(0u32.to_be_bytes());
        auth.extend([0u8; 16]); // aaguid
        auth.extend((self.id.len() as u16).to_be_bytes());
        auth.extend(&self.id);
        auth.extend(cose);
        let mut att = vec![0xa3, 0x63];
        att.extend(b"fmt");
        att.push(0x64);
        att.extend(b"none");
        att.push(0x67);
        att.extend(b"attStmt");
        att.push(0xa0);
        att.push(0x68);
        att.extend(b"authData");
        cbor_bytes(&mut att, &auth);
        json!({
            "id": B64.encode(&self.id),
            "rawId": B64.encode(&self.id),
            "type": "public-key",
            "response": {
                "attestationObject": B64.encode(att),
                "clientDataJSON": B64.encode(Self::client_data("webauthn.create", options, origin)),
            },
        })
    }

    /// The answer to `navigator.credentials.get`; `handle` is the user handle it returns.
    fn get(&mut self, options: &Value, origin: &str, handle: &[u8]) -> Value {
        self.counter += 1;
        let mut auth = Self::rp_hash().to_vec();
        auth.push(0x01 | 0x04);
        auth.extend(self.counter.to_be_bytes());
        let client = Self::client_data("webauthn.get", options, origin);
        let mut msg = auth.clone();
        msg.extend(Sha256::digest(&client));
        let sig = self.key.sign(&SystemRandom::new(), &msg).unwrap();
        json!({
            "id": B64.encode(&self.id),
            "rawId": B64.encode(&self.id),
            "type": "public-key",
            "response": {
                "authenticatorData": B64.encode(auth),
                "clientDataJSON": B64.encode(client),
                "signature": B64.encode(sig.as_ref()),
                "userHandle": B64.encode(handle),
            },
        })
    }
}

fn handle(s: &Session) -> Vec<u8> {
    (0..16)
        .map(|i| u8::from_str_radix(&s.user.user_id[i * 2..i * 2 + 2], 16).unwrap())
        .collect()
}

/// Registers `soft` for `s` and returns the passkey id.
async fn register(t: &TestServer, s: &Session, soft: &Soft, name: &str) -> i64 {
    let (st, _, v) = t
        .call(post("/api/passkeys/register/start", json!({}), s))
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let cred = soft.create(&v["options"], ORIGIN);
    let body = json!({ "challenge_id": v["challenge_id"], "credential": cred, "name": name });
    let (st, _, v) = t.call(post("/api/passkeys/register/finish", body, s)).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["name"], name);
    v["id"].as_i64().unwrap()
}

/// Passkey sign in with no cookie; the answer is signed for `origin`.
async fn sign_in(
    t: &TestServer,
    soft: &mut Soft,
    handle: &[u8],
    origin: &str,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let (st, _, v) = t
        .call(request(
            Method::POST,
            "/api/passkeys/login/start",
            None,
            None,
        ))
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let cred = soft.get(&v["options"], origin, handle);
    let body = json!({ "challenge_id": v["challenge_id"], "credential": cred });
    t.call(request(
        Method::POST,
        "/api/passkeys/login/finish",
        Some(&body),
        None,
    ))
    .await
}

#[tokio::test]
async fn register_then_sign_in_makes_a_normal_session() {
    let t = TestServer::new().await;
    let alice = t.owner().await;
    let mut soft = Soft::new();
    let id = register(&t, &alice, &soft, "Test key").await;

    let (st, _, v) = t.call(get("/api/passkeys", &alice)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["enabled"], true);
    assert_eq!(v["passkeys"][0]["name"], "Test key");
    assert!(v["passkeys"][0]["last_used_at"].is_null());

    let (st, h, v) = sign_in(&t, &mut soft, &handle(&alice), ORIGIN).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["user"]["user_id"], alice.user.user_id);
    let cookie = common::set_cookie(&h);
    let flags = h.get("set-cookie").unwrap().to_str().unwrap();
    assert!(
        flags.contains("HttpOnly") && flags.contains("SameSite=Strict"),
        "{flags}"
    );
    let me = Session {
        cookie,
        user: alice.user.clone(),
    };
    let (st, _, v) = t.call(get("/api/me", &me)).await;
    assert_eq!(st, StatusCode::OK, "{v}");

    let (_, _, v) = t.call(get("/api/passkeys", &me)).await;
    assert!(v["passkeys"][0]["last_used_at"].is_i64());

    // A second sign in works (the counter moved forward), and a rename sticks.
    let (st, _, _) = sign_in(&t, &mut soft, &handle(&alice), ORIGIN).await;
    assert_eq!(st, StatusCode::OK);
    let req = request(
        Method::PATCH,
        &format!("/api/passkeys/{id}"),
        Some(&json!({ "name": "Renamed" })),
        Some(&me.cookie),
    );
    assert_eq!(t.call(req).await.0, StatusCode::NO_CONTENT);
    let (_, _, v) = t.call(get("/api/passkeys", &me)).await;
    assert_eq!(v["passkeys"][0]["name"], "Renamed");
}

#[tokio::test]
async fn wrong_origin_and_replay_are_refused() {
    let t = TestServer::new().await;
    let alice = t.owner().await;
    let mut soft = Soft::new();
    register(&t, &alice, &soft, "k").await;

    let (st, h, _) = sign_in(&t, &mut soft, &handle(&alice), "http://evil.example").await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert!(h.get("set-cookie").is_none());

    // Registering from another origin fails too.
    let (_, _, v) = t
        .call(post("/api/passkeys/register/start", json!({}), &alice))
        .await;
    let other = Soft::new();
    let cred = other.create(&v["options"], "http://evil.example");
    let body = json!({ "challenge_id": v["challenge_id"], "credential": cred });
    let (st, _, _) = t
        .call(post("/api/passkeys/register/finish", body, &alice))
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // A challenge works once, even for the right answer.
    let (_, _, v) = t
        .call(request(
            Method::POST,
            "/api/passkeys/login/start",
            None,
            None,
        ))
        .await;
    let cred = soft.get(&v["options"], ORIGIN, &handle(&alice));
    let body = json!({ "challenge_id": v["challenge_id"], "credential": cred });
    let finish = |b: &Value| request(Method::POST, "/api/passkeys/login/finish", Some(b), None);
    assert_eq!(t.call(finish(&body)).await.0, StatusCode::OK);
    assert_eq!(t.call(finish(&body)).await.0, StatusCode::UNAUTHORIZED);

    // The Origin header is checked as for every change.
    let mut req = finish(&body);
    req.headers_mut()
        .insert("origin", "http://evil.example".parse().unwrap());
    assert_eq!(t.call(req).await.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn deleted_unknown_and_banned_cannot_sign_in() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let bob = t.member("bob").await;
    let mut soft = Soft::new();
    let id = register(&t, &bob, &soft, "bobs").await;

    // Another account cannot rename or remove it.
    let del = |s: &Session| {
        request(
            Method::DELETE,
            &format!("/api/passkeys/{id}"),
            None,
            Some(&s.cookie),
        )
    };
    assert_eq!(t.call(del(&owner)).await.0, StatusCode::NOT_FOUND);

    // A key nobody registered.
    let mut stranger = Soft::new();
    let (st, _, _) = sign_in(&t, &mut stranger, &handle(&bob), ORIGIN).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    // A known key under someone else's user handle.
    let (st, _, _) = sign_in(&t, &mut soft, &handle(&owner), ORIGIN).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    // Banned: refused, and it works again after the ban is lifted.
    t.state
        .store
        .set_banned(&bob.user.user_id, true, 1)
        .await
        .unwrap();
    let (st, h, _) = sign_in(&t, &mut soft, &handle(&bob), ORIGIN).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert!(h.get("set-cookie").is_none());
    t.state
        .store
        .set_banned(&bob.user.user_id, false, 2)
        .await
        .unwrap();
    assert_eq!(
        sign_in(&t, &mut soft, &handle(&bob), ORIGIN).await.0,
        StatusCode::OK
    );

    // Removed: refused.
    let bob = t.session(&bob.user).await; // the ban ended his sessions
    assert_eq!(t.call(del(&bob)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(
        sign_in(&t, &mut soft, &handle(&bob), ORIGIN).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn registering_needs_a_session_and_skips_known_keys() {
    let t = TestServer::new().await;
    let alice = t.owner().await;
    let body = json!({});
    let (st, _, _) = t
        .call(request(
            Method::POST,
            "/api/passkeys/register/start",
            Some(&body),
            None,
        ))
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let soft = Soft::new();
    register(&t, &alice, &soft, "one").await;
    // The same authenticator again is refused (webauthn-rs checks the exclude list).
    let (_, _, v) = t
        .call(post("/api/passkeys/register/start", json!({}), &alice))
        .await;
    assert_eq!(
        v["options"]["publicKey"]["excludeCredentials"][0]["id"],
        B64.encode(&soft.id)
    );
    let cred = soft.create(&v["options"], ORIGIN);
    let body = json!({ "challenge_id": v["challenge_id"], "credential": cred });
    let (st, _, _) = t
        .call(post("/api/passkeys/register/finish", body, &alice))
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}
