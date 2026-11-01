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
    // The refused owner invite was not spent.
    let invites = t.state.store.invites().await.unwrap();
    assert_eq!(invites.len(), 2);
    assert_eq!(invites.iter().map(|i| i.uses).sum::<u32>(), 1);
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

#[tokio::test]
async fn login_ip_limit_comes_before_the_username_limit() {
    let t = TestServer::new().await;
    t.member("alice").await;
    // Bad-format names fail before argon2 but still spend the address's bucket (20).
    let junk = login_body("x", "test-password-2");
    for _ in 0..20 {
        let (s, _, _) = t
            .call(request(Method::POST, "/api/login", Some(&junk), None))
            .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    // The right password for a fresh username bucket is still refused: the address is spent.
    let good = login_body("alice", PASSWORD);
    let (s, h, v) = t
        .call(request(Method::POST, "/api/login", Some(&good), None))
        .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{v}");
    assert!(h.get(header::RETRY_AFTER).is_some());
    let mut other = request(Method::POST, "/api/login", Some(&good), None);
    other
        .headers_mut()
        .insert("cf-connecting-ip", "203.0.113.9".parse().unwrap());
    assert_eq!(t.call(other).await.0, StatusCode::OK);
}

#[tokio::test]
async fn banned_user_cannot_log_in() {
    let t = TestServer::new().await;
    let a = t.member("alice").await;
    t.state
        .store
        .set_banned(&a.user.user_id, true, vesper_store::now_ms())
        .await
        .unwrap();
    let (s, h, v) = t
        .call(request(
            Method::POST,
            "/api/login",
            Some(&login_body("alice", PASSWORD)),
            None,
        ))
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(v["message"], "wrong username or password");
    assert!(h.get(header::SET_COOKIE).is_none());
}

#[tokio::test]
async fn https_cookie_is_secure_host_prefixed() {
    let mut t = TestServer::new().await;
    let mut cfg = (*t.state.cfg).clone();
    cfg.secure_cookies = true;
    t.state.cfg = std::sync::Arc::new(cfg);
    let a = t.member("alice").await;
    assert!(
        a.cookie.starts_with("__Host-vesper_session="),
        "{}",
        a.cookie
    );
    let (_, h, _) = t
        .call(request(
            Method::POST,
            "/api/login",
            Some(&login_body("alice", PASSWORD)),
            None,
        ))
        .await;
    let c = h[header::SET_COOKIE].to_str().unwrap();
    for part in ["Secure", "HttpOnly", "SameSite=Strict", "Path=/"] {
        assert!(c.contains(part), "{c}");
    }
    assert!(!c.to_ascii_lowercase().contains("domain="), "{c}");
    assert_eq!(t.call(get("/api/me", &a)).await.0, StatusCode::OK);
    // The http cookie name does not count on https.
    let plain = a.cookie.replacen("__Host-", "", 1);
    let req = request(Method::GET, "/api/me", None, Some(&plain));
    assert_eq!(t.call(req).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn patch_without_origin_is_403() {
    let t = TestServer::new().await;
    let a = t.member("alice").await;
    let req = Request::builder()
        .method(Method::PATCH)
        .uri("/api/me")
        .header(header::COOKIE, &a.cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "display_name": "Mallory" }).to_string()))
        .unwrap();
    let (s, _, v) = t.call(req).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (_, _, me) = t.call(get("/api/me", &a)).await;
    assert_eq!(me["user"]["display_name"], "alice");
}

#[tokio::test]
async fn signup_drops_a_presented_session() {
    let t = TestServer::new().await;
    let old = t.member("alice").await;
    let code = t.invite(&["invite"]).await;
    let (s, _, _) = t
        .call(post("/api/signup", signup_body(&code, "bob"), &old))
        .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(
        t.call(get("/api/me", &old)).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn revoking_the_current_session_clears_the_cookie() {
    let t = TestServer::new().await;
    let a = t.member("alice").await;
    let (_, _, me) = t.call(get("/api/me", &a)).await;
    let path = format!("/api/sessions/{}", me["session_id"]);
    let (s, h, _) = t
        .call(request(Method::DELETE, &path, None, Some(&a.cookie)))
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(
        h[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(t.call(get("/api/me", &a)).await.0, StatusCode::UNAUTHORIZED);
}

/// The `(METHOD, /api path)` of every route in the server's source that is not GET, read from
/// the `.route("...", post(..).delete(..))` lines, so a new route cannot miss the Origin test.
fn mutating_routes() -> Vec<(String, String)> {
    let sources = [
        include_str!("../src/api_accounts.rs"),
        include_str!("../src/api_admin.rs"),
        include_str!("../src/api_rooms.rs"),
        include_str!("../src/api_messages.rs"),
        include_str!("../src/uploads.rs"),
    ];
    let mut out = Vec::new();
    for line in sources.iter().flat_map(|s| s.lines()) {
        let Some(rest) = line.trim().strip_prefix(".route(\"") else {
            continue;
        };
        let (path, handlers) = rest.split_once('"').unwrap();
        for m in ["post", "put", "patch", "delete"] {
            if handlers.contains(&format!("{m}(")) {
                out.push((m.to_uppercase(), format!("/api{path}")));
            }
        }
    }
    out.sort();
    out
}

/// CSRF: every non-GET `/api` route refuses a missing, foreign or `null` Origin before the handler
/// runs, even for the owner with a valid session. (`/ws` is in tests/signaling.rs.)
#[tokio::test]
async fn every_mutating_route_needs_our_origin() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let routes = mutating_routes();
    assert!(routes.len() >= 25, "parsed too few routes: {routes:?}");
    let id = &owner.user.user_id;
    for (method, path) in &routes {
        let path = path.replace("{id}", id);
        for origin in [None, Some("http://evil.example"), Some("null")] {
            let mut b = Request::builder()
                .method(method.as_str())
                .uri(&path)
                .header(header::COOKIE, &owner.cookie)
                .header(header::CONTENT_TYPE, "application/json");
            if let Some(o) = origin {
                b = b.header(header::ORIGIN, o);
            }
            let (s, _, v) = t.call(b.body(Body::from("{}")).unwrap()).await;
            assert_eq!(
                (s, v["code"].as_str()),
                (StatusCode::FORBIDDEN, Some("forbidden")),
                "{method} {path} {origin:?}: {v}"
            );
        }
    }
    // The owner's session still works: nothing above logged it out.
    assert_eq!(t.call(get("/api/me", &owner)).await.0, StatusCode::OK);
}

/// Invite preview and signup share one limit per client address: 10, then 1 every 6 minutes.
#[tokio::test]
async fn invite_preview_and_signup_are_rate_limited() {
    let t = TestServer::new().await;
    let from = |ip: &'static str, mut req: Request<Body>| {
        req.headers_mut()
            .insert("x-forwarded-for", ip.parse().unwrap());
        req
    };
    let preview = |ip: &'static str| {
        let req = request(Method::GET, "/api/invites/preview?code=nope", None, None);
        t.call(from(ip, req))
    };
    for i in 0..10 {
        let (s, _, v) = preview("203.0.113.7").await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{i}: {v}");
    }
    let (s, h, v) = preview("203.0.113.7").await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited"))
    );
    assert!(h.contains_key("retry-after"));
    // Signup draws on the same bucket, even with a good invite.
    let code = t.invite(&["invite"]).await;
    let body = json!({
        "invite": code, "username": "alice", "password": PASSWORD, "display_name": "alice",
    });
    let signup = |ip| from(ip, request(Method::POST, "/api/signup", Some(&body), None));
    assert_eq!(
        t.call(signup("203.0.113.7")).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(users(&t).await, 0);
    // Another address is not held back.
    assert_eq!(t.call(signup("198.51.100.9")).await.0, StatusCode::CREATED);
}
