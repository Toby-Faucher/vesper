//! The account routes: signup, login, sessions, profile, Origin check and body cap.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{PASSWORD, TestServer, get, post, request, set_cookie};
use serde_json::{Value, json};

fn signup_body(code: &str, username: &str) -> Value {
    json!({ "invite": code, "username": username, "password": PASSWORD, "display_name": "A" })
}

fn login_body(username: &str, password: &str) -> Value {
    json!({ "username": username, "password": password })
}

async fn users(t: &TestServer) -> usize {
    t.state.store.users().await.unwrap().len()
}

#[tokio::test]
async fn signup_with_invite_sets_strict_cookie() {
    let t = TestServer::new().await;
    let code = t.invite(&["invite"]).await;
    let req = request(
        Method::POST,
        "/api/signup",
        Some(&signup_body(&code, "Alice_1")),
        None,
    );
    let (status, h, v) = t.call(req).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let c = h.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(c.starts_with("vesper_session="), "{c}");
    assert!(
        c.contains("HttpOnly") && c.contains("SameSite=Strict"),
        "{c}"
    );
    assert!(!c.contains("Secure"), "{c}");
    assert_eq!(v["user"]["role"], "member");
    let cookie = set_cookie(&h);
    let (status, _, v) = t
        .call(request(Method::GET, "/api/me", None, Some(&cookie)))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["user"]["username"], "alice_1");
}

#[tokio::test]
async fn signup_with_used_invite_is_rejected() {
    let t = TestServer::new().await;
    let code = t.invite(&["invite"]).await;
    t.signup(&code, "alice").await;
    let again = request(
        Method::POST,
        "/api/signup",
        Some(&signup_body(&code, "bob")),
        None,
    );
    let (status, _, used) = t.call(again).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(used["code"], "invite_invalid");
    let fresh = t.invite(&["invite"]).await;
    let last = if fresh.ends_with('0') { '1' } else { '0' };
    let changed = format!("{}{last}", &fresh[..63]);
    let req = request(
        Method::POST,
        "/api/signup",
        Some(&signup_body(&changed, "bob")),
        None,
    );
    let (status, _, v) = t.call(req).await;
    assert_eq!((status, v), (StatusCode::NOT_FOUND, used.clone()));
    let (status, _, v) = t
        .call(request(
            Method::GET,
            &format!("/api/invites/preview?code={changed}"),
            None,
            None,
        ))
        .await;
    assert_eq!((status, v), (StatusCode::NOT_FOUND, used));
    let (status, _, v) = t
        .call(request(
            Method::GET,
            &format!("/api/invites/preview?code={fresh}"),
            None,
            None,
        ))
        .await;
    assert_eq!((status, v), (StatusCode::OK, json!({ "role": "member" })));
    assert_eq!(users(&t).await, 1);
}

#[tokio::test]
async fn second_owner_link_cannot_make_a_second_owner() {
    let t = TestServer::new().await;
    let first = t.invite(&["create-owner"]).await;
    let second = t.invite(&["create-owner"]).await;
    t.signup(&first, "owner").await;
    let req = request(
        Method::POST,
        "/api/signup",
        Some(&signup_body(&second, "owner2")),
        None,
    );
    let (status, _, v) = t.call(req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(v["code"], "invite_invalid");
    let preview = format!("/api/invites/preview?code={second}");
    let (status, _, _) = t.call(request(Method::GET, &preview, None, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(users(&t).await, 1);
}

#[tokio::test]
async fn login_wrong_password_and_unknown_user_look_the_same() {
    let t = TestServer::new().await;
    t.member("alice").await;
    let wrong = login_body("alice", "test-password-2");
    let (s1, _, v1) = t
        .call(request(Method::POST, "/api/login", Some(&wrong), None))
        .await;
    let nobody = login_body("nobody", "test-password-2");
    let (s2, _, v2) = t
        .call(request(Method::POST, "/api/login", Some(&nobody), None))
        .await;
    assert_eq!(s1, StatusCode::UNAUTHORIZED);
    assert_eq!((s1, &v1), (s2, &v2));
    assert_eq!(v1["message"], "wrong username or password");
    t.login("ALICE").await;
}

#[tokio::test]
async fn login_is_rate_limited() {
    let t = TestServer::new().await;
    t.member("alice").await;
    let wrong = login_body("alice", "test-password-2");
    for _ in 0..5 {
        let (s, _, _) = t
            .call(request(Method::POST, "/api/login", Some(&wrong), None))
            .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    let (s, h, v) = t
        .call(request(Method::POST, "/api/login", Some(&wrong), None))
        .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{v}");
    assert_eq!(v["code"], "rate_limited");
    let wait: u64 = h[header::RETRY_AFTER].to_str().unwrap().parse().unwrap();
    assert!((1..=60).contains(&wait), "{wait}");
    // Another address still gets its own bucket for that username.
    let mut other = request(Method::POST, "/api/login", Some(&wrong), None);
    other
        .headers_mut()
        .insert("x-forwarded-for", "203.0.113.9".parse().unwrap());
    let (s, _, _) = t.call(other).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_mints_new_token_and_drops_presented_one() {
    let t = TestServer::new().await;
    let old = t.member("alice").await;
    let body = login_body("alice", PASSWORD);
    let (s, h, _) = t.call(post("/api/login", body, &old)).await;
    assert_eq!(s, StatusCode::OK);
    let new = set_cookie(&h);
    assert_ne!(new, old.cookie);
    let (s, _, _) = t.call(get("/api/me", &old)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, _) = t
        .call(request(Method::GET, "/api/me", None, Some(&new)))
        .await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn post_without_or_with_wrong_origin_is_403() {
    let t = TestServer::new().await;
    t.member("alice").await;
    let alice = t.state.store.login_row("alice").await.unwrap().unwrap();
    let before = t
        .state
        .store
        .sessions(&alice.user_id, 0)
        .await
        .unwrap()
        .len();
    for origin in [None, Some("http://evil.example"), Some("null")] {
        let mut b = Request::builder()
            .method(Method::POST)
            .uri("/api/login")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(o) = origin {
            b = b.header(header::ORIGIN, o);
        }
        let req = b
            .body(Body::from(login_body("alice", PASSWORD).to_string()))
            .unwrap();
        let (s, h, v) = t.call(req).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{origin:?}");
        assert_eq!(v["code"], "forbidden");
        assert!(h.get(header::SET_COOKIE).is_none());
    }
    let after = t
        .state
        .store
        .sessions(&alice.user_id, 0)
        .await
        .unwrap()
        .len();
    assert_eq!(before, after);
}

#[tokio::test]
async fn password_change_drops_other_sessions() {
    let t = TestServer::new().await;
    t.member("alice").await;
    let a = t.login("alice").await;
    let b = t.login("alice").await;
    let wrong = json!({ "old": "test-password-2", "new": "test-password-3" });
    let (s, _, _) = t.call(post("/api/me/password", wrong, &a)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let short = json!({ "old": PASSWORD, "new": "short" });
    let (s, _, _) = t.call(post("/api/me/password", short, &a)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let ok = json!({ "old": PASSWORD, "new": "test-password-3" });
    let (s, _, _) = t.call(post("/api/me/password", ok, &a)).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(t.call(get("/api/me", &b)).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(t.call(get("/api/me", &a)).await.0, StatusCode::OK);
    let (s, _, _) = t
        .call(request(
            Method::POST,
            "/api/login",
            Some(&login_body("alice", "test-password-3")),
            None,
        ))
        .await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn revoke_device() {
    let t = TestServer::new().await;
    let a = t.member("alice").await;
    let b = t.login("alice").await;
    let (s, _, v) = t.call(get("/api/sessions", &a)).await;
    assert_eq!(s, StatusCode::OK);
    let list = v.as_array().unwrap();
    assert_eq!(list.len(), 2);
    let current: Vec<_> = list.iter().filter(|d| d["current"] == true).collect();
    assert_eq!(current.len(), 1);
    let other = list.iter().find(|d| d["current"] == false).unwrap()["id"].clone();
    let del = |id: &Value, s: &common::Session| {
        request(
            Method::DELETE,
            &format!("/api/sessions/{id}"),
            None,
            Some(&s.cookie),
        )
    };
    assert_eq!(t.call(del(&other, &a)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(t.call(get("/api/me", &b)).await.0, StatusCode::UNAUTHORIZED);
    let c = t.member("carol").await;
    let (_, _, v) = t.call(get("/api/sessions", &c)).await;
    let carols = v[0]["id"].clone();
    assert_eq!(t.call(del(&carols, &a)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.call(get("/api/me", &c)).await.0, StatusCode::OK);
    // Logout ends the session and clears the cookie.
    let (s, h, _) = t.call(post("/api/logout", json!({}), &c)).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(
        h[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(t.call(get("/api/me", &c)).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn profile_patch_validates() {
    let t = TestServer::new().await;
    let a = t.member("alice").await;
    let patch = |v: Value| request(Method::PATCH, "/api/me", Some(&v), Some(&a.cookie));
    let (s, _, v) = t.call(patch(json!({ "accent": "red" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["code"], "bad_request");
    let (s, _, _) = t
        .call(patch(json!({ "display_name": "Al\u{200B}ice" })))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, v) = t
        .call(patch(
            json!({ "display_name": "Alice A", "accent": "#7C8CFF" }),
        ))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["display_name"], "Alice A");
    assert_eq!(v["accent"], "#7c8cff");
    assert_eq!(v["username"], "alice");
    let (_, _, v) = t.call(patch(json!({}))).await;
    assert_eq!(v["display_name"], "Alice A");
}

#[tokio::test]
async fn body_over_32k_is_413() {
    let t = TestServer::new().await;
    let big = login_body("alice", &"x".repeat(33 * 1024));
    let (s, _, v) = t
        .call(request(Method::POST, "/api/login", Some(&big), None))
        .await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE, "{v}");
    assert_eq!(v["code"], "too_large");
}

#[tokio::test]
async fn signed_out_calls_are_401() {
    let t = TestServer::new().await;
    for (m, p) in [(Method::GET, "/api/me"), (Method::GET, "/api/sessions")] {
        let (s, _, v) = t.call(request(m, p, None, None)).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        assert_eq!(v["code"], "unauthorized");
    }
    let junk = request(Method::GET, "/api/me", None, Some("vesper_session=nothex"));
    assert_eq!(t.call(junk).await.0, StatusCode::UNAUTHORIZED);
}
