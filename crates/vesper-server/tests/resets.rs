//! Password reset links: who may make them, and what using one does.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{ORIGIN, PASSWORD, Session, TestServer, get, post, request};
use serde_json::{Value, json};
use vesper_server::auth::{hash_token, new_token};
use vesper_store::now_ms;

const NEW_PASSWORD: &str = "test-password-2";

async fn moderator(t: &TestServer, name: &str) -> Session {
    let code = t.invite(&["invite", "--role", "moderator"]).await;
    t.signup(&code, name).await
}

fn link_req(by: &Session, id: &str, password: &str, ip: &str) -> Request<Body> {
    let mut req = post(
        &format!("/api/users/{id}/reset"),
        json!({ "password": password }),
        by,
    );
    req.headers_mut()
        .insert("x-forwarded-for", ip.parse().unwrap());
    req
}

/// `POST /api/users/{id}/reset` as `by` (who types `PASSWORD`): the status and the code from the
/// link, if any.
async fn make_link(t: &TestServer, by: &Session, id: &str) -> (StatusCode, Option<String>) {
    let (s, _, v) = t.call(link_req(by, id, PASSWORD, "127.0.0.1")).await;
    let code = v["url"].as_str().map(|url| {
        let code = url
            .strip_prefix(&format!("{ORIGIN}/reset#"))
            .unwrap_or_else(|| panic!("not a reset link: {url}"));
        assert_eq!(code.len(), 64);
        code.to_string()
    });
    if s == StatusCode::CREATED {
        assert!(v["expires_at"].as_i64().unwrap() > now_ms());
    }
    (s, code)
}

fn reset_req(code: &str, password: &str) -> Request<Body> {
    let body = json!({ "code": code, "password": password });
    request(Method::POST, "/api/reset", Some(&body), None)
}

async fn reset(t: &TestServer, code: &str, password: &str) -> (StatusCode, Value) {
    let (s, _, v) = t.call(reset_req(code, password)).await;
    (s, v)
}

async fn login_status(t: &TestServer, username: &str, password: &str) -> StatusCode {
    let body = json!({ "username": username, "password": password });
    t.call(request(Method::POST, "/api/login", Some(&body), None))
        .await
        .0
}

async fn me_status(t: &TestServer, s: &Session) -> StatusCode {
    t.call(get("/api/me", s)).await.0
}

#[tokio::test]
async fn who_may_make_a_reset_link() {
    use StatusCode as S;
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let mod1 = moderator(&t, "mod1").await;
    let mod2 = moderator(&t, "mod2").await;
    let mem1 = t.member("mem1").await;
    let mem2 = t.member("mem2").await;
    let (o, m1, m2, a, b) = (&owner, &mod1, &mod2, &mem1, &mem2);
    let id = |s: &Session| s.user.user_id.clone();
    for (by, target, want) in [
        (o, id(o), S::BAD_REQUEST),
        (o, id(m1), S::CREATED),
        (o, id(a), S::CREATED),
        (m1, id(o), S::FORBIDDEN),
        (m1, id(m1), S::BAD_REQUEST),
        (m1, id(m2), S::FORBIDDEN),
        (m1, id(a), S::CREATED),
        (a, id(o), S::FORBIDDEN),
        (a, id(m1), S::FORBIDDEN),
        (a, id(a), S::FORBIDDEN),
        (a, id(b), S::FORBIDDEN),
        (o, "0".repeat(32), S::NOT_FOUND),
        (m1, "0".repeat(32), S::NOT_FOUND),
    ] {
        let (s, code) = make_link(&t, by, &target).await;
        assert_eq!(s, want, "{} -> {target}", by.user.username);
        assert_eq!(code.is_some(), want == S::CREATED);
    }
    // Each target above got at most one link, made by whoever was allowed.
    // Signed out: 401.
    let req = request(
        Method::POST,
        &format!("/api/users/{}/reset", id(a)),
        None,
        None,
    );
    assert_eq!(t.call(req).await.0, S::UNAUTHORIZED);
    // Nobody was signed out by any of that.
    for s in [o, m1, m2, a, b] {
        assert_eq!(me_status(&t, s).await, S::OK);
    }
}

#[tokio::test]
async fn reset_sets_the_password_and_ends_every_session() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let bob = t.member("bob").await;
    let bob2 = t.login("bob").await;
    let alice = t.member("alice").await;
    let (s, code) = make_link(&t, &owner, &bob.user.user_id).await;
    assert_eq!(s, StatusCode::CREATED);
    let code = code.unwrap();
    // Making a link changes nothing yet.
    assert_eq!(me_status(&t, &bob).await, StatusCode::OK);

    assert_eq!(
        reset(&t, &code, NEW_PASSWORD).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(me_status(&t, &bob).await, StatusCode::UNAUTHORIZED);
    assert_eq!(me_status(&t, &bob2).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        login_status(&t, "bob", PASSWORD).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(login_status(&t, "bob", NEW_PASSWORD).await, StatusCode::OK);
    // Others are untouched.
    assert_eq!(me_status(&t, &alice).await, StatusCode::OK);
    assert_eq!(me_status(&t, &owner).await, StatusCode::OK);
    // Single use: a replay fails and changes nothing.
    let (s, v) = reset(&t, &code, "test-password-3").await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("reset_invalid"))
    );
    assert_eq!(login_status(&t, "bob", NEW_PASSWORD).await, StatusCode::OK);
}

/// Unknown, malformed, used, replaced and expired codes all get the same answer.
#[tokio::test]
async fn every_bad_code_looks_the_same() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let bob = t.member("bob").await;
    let carol = t.member("carol").await;
    let id = &bob.user.user_id;
    let replaced = make_link(&t, &owner, id).await.1.unwrap();
    let used = make_link(&t, &owner, id).await.1.unwrap();
    assert_eq!(
        reset(&t, &used, NEW_PASSWORD).await.0,
        StatusCode::NO_CONTENT
    );
    let (expired, hash) = new_token();
    t.state
        .store
        .create_reset(&carol.user.user_id, &hash, now_ms() - 1, None)
        .await
        .unwrap();
    let unknown = new_token().0;
    let mut answers = Vec::new();
    for code in [unknown.as_str(), "nope", "", &replaced, &used, &expired] {
        answers.push(reset(&t, code, "test-password-3").await);
    }
    for a in &answers {
        assert_eq!(a, &answers[0]);
    }
    assert_eq!(answers[0].0, StatusCode::NOT_FOUND);
    assert_eq!(answers[0].1["code"], "reset_invalid");
    assert_eq!(answers[0].1["message"], "reset link not valid");
    // Neither bob nor carol got the last password.
    assert_eq!(login_status(&t, "bob", NEW_PASSWORD).await, StatusCode::OK);
    assert_eq!(login_status(&t, "carol", PASSWORD).await, StatusCode::OK);
    assert_eq!(me_status(&t, &carol).await, StatusCode::OK);
}

#[tokio::test]
async fn a_new_link_replaces_the_old_one() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let m = moderator(&t, "mod").await;
    let bob = t.member("bob").await;
    let first = make_link(&t, &owner, &bob.user.user_id).await.1.unwrap();
    let second = make_link(&t, &m, &bob.user.user_id).await.1.unwrap();
    assert_eq!(
        reset(&t, &first, NEW_PASSWORD).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(login_status(&t, "bob", PASSWORD).await, StatusCode::OK);
    assert_eq!(
        reset(&t, &second, NEW_PASSWORD).await.0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn password_rules_apply_and_keep_the_code() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let bob = t.member("bob").await;
    let code = make_link(&t, &owner, &bob.user.user_id).await.1.unwrap();
    for bad in ["short", &"x".repeat(129)] {
        let (s, v) = reset(&t, &code, bad).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    }
    assert_eq!(me_status(&t, &bob).await, StatusCode::OK);
    assert_eq!(
        reset(&t, &code, NEW_PASSWORD).await.0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn reset_routes_need_our_origin() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let bob = t.member("bob").await;
    let code = make_link(&t, &owner, &bob.user.user_id).await.1.unwrap();
    let path = format!("/api/users/{}/reset", bob.user.user_id);
    for origin in [None, Some("http://evil.example"), Some("null")] {
        for (uri, body, cookie) in [
            (
                "/api/reset",
                json!({ "code": code, "password": NEW_PASSWORD }),
                None,
            ),
            (
                path.as_str(),
                json!({ "password": PASSWORD }),
                Some(owner.cookie.as_str()),
            ),
        ] {
            let mut b = Request::builder().method(Method::POST).uri(uri);
            if let Some(o) = origin {
                b = b.header(header::ORIGIN, o);
            }
            if let Some(c) = cookie {
                b = b.header(header::COOKIE, c);
            }
            let req = b
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            assert_eq!(
                t.call(req).await.0,
                StatusCode::FORBIDDEN,
                "{uri} {origin:?}"
            );
        }
    }
    // The code still works from our origin, and is the only one (no new link was made above).
    assert_eq!(
        reset(&t, &code, NEW_PASSWORD).await.0,
        StatusCode::NO_CONTENT
    );
}

/// 10 tries per address, then 1 every 6 minutes; another address has its own budget.
#[tokio::test]
async fn reset_is_rate_limited() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let bob = t.member("bob").await;
    let code = make_link(&t, &owner, &bob.user.user_id).await.1.unwrap();
    let from = |ip: &'static str, code: &str| {
        let mut req = reset_req(code, NEW_PASSWORD);
        req.headers_mut()
            .insert("x-forwarded-for", ip.parse().unwrap());
        req
    };
    let junk = new_token().0;
    for i in 0..10 {
        let (s, _, v) = t.call(from("203.0.113.7", &junk)).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{i}: {v}");
    }
    // Even the right code is held back now.
    let (s, h, v) = t.call(from("203.0.113.7", &code)).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited"))
    );
    assert!(h.contains_key(header::RETRY_AFTER));
    assert_eq!(me_status(&t, &bob).await, StatusCode::OK);
    assert_eq!(
        t.call(from("198.51.100.9", &code)).await.0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn cli_reset_password_works_for_the_owner() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let link = vesper_server::cli::run(&args(&["reset-password", "Owner"]), &t.state.cfg)
        .await
        .unwrap()
        .unwrap();
    let code = link.strip_prefix(&format!("{ORIGIN}/reset#")).unwrap();
    assert!(hash_token(code).is_some());
    assert_eq!(
        reset(&t, code, NEW_PASSWORD).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(me_status(&t, &owner).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        login_status(&t, "owner", NEW_PASSWORD).await,
        StatusCode::OK
    );
    for bad in [
        &["reset-password"][..],
        &["reset-password", "nobody"],
        &["reset-password", "a", "b"],
    ] {
        assert!(
            vesper_server::cli::run(&args(bad), &t.state.cfg)
                .await
                .is_err(),
            "{bad:?}"
        );
    }
}

/// The issuer types their own password: a wrong or missing one makes no link, and wrong ones
/// draw on the issuer's sign-in limit.
#[tokio::test]
async fn making_a_link_needs_your_own_password() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let bob = t.member("bob").await;
    let id = &bob.user.user_id;
    for i in 0..5 {
        let (s, _, v) = t
            .call(link_req(&owner, id, "test-password-9", "203.0.113.7"))
            .await;
        assert_eq!(
            (s, v["message"].as_str()),
            (StatusCode::FORBIDDEN, Some("wrong password")),
            "{i}"
        );
        assert!(v.get("url").is_none());
    }
    let (s, _, _) = t.call(link_req(&owner, id, PASSWORD, "203.0.113.7")).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    let (s, _, _) = t
        .call(post(&format!("/api/users/{id}/reset"), json!({}), &owner))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(t.state.store.resets(now_ms()).await.unwrap().is_empty());
    // The right password from another address works.
    assert_eq!(make_link(&t, &owner, id).await.0, StatusCode::CREATED);
}

/// 10 links per issuer, then 1 every 6 minutes, wherever they come from.
#[tokio::test]
async fn making_links_is_rate_limited_per_issuer() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let m = moderator(&t, "mod").await;
    let bob = t.member("bob").await;
    let id = &bob.user.user_id;
    for i in 0..10 {
        let ip: &'static str = Box::leak(format!("198.51.100.{i}").into_boxed_str());
        let (s, _, v) = t.call(link_req(&m, id, PASSWORD, ip)).await;
        assert_eq!(s, StatusCode::CREATED, "{i}: {v}");
    }
    let (s, h, v) = t.call(link_req(&m, id, PASSWORD, "198.51.100.99")).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited"))
    );
    assert!(h.contains_key(header::RETRY_AFTER));
    // The owner has a budget of their own.
    assert_eq!(make_link(&t, &owner, id).await.0, StatusCode::CREATED);
}

/// The owner sees the latest reset of each account: who made it, when, and whether it was used.
#[tokio::test]
async fn the_owner_sees_who_reset_whom() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let m = moderator(&t, "mod").await;
    let bob = t.member("bob").await;
    let code = make_link(&t, &m, &bob.user.user_id).await.1.unwrap();
    assert_eq!(
        reset(&t, &code, NEW_PASSWORD).await.0,
        StatusCode::NO_CONTENT
    );
    let (s, _, v) = t.call(get("/api/resets", &owner)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = &v.as_array().unwrap()[0];
    assert_eq!(r["user_id"], bob.user.user_id.as_str());
    assert_eq!(r["created_by"], m.user.user_id.as_str());
    assert!(r["used_at"].as_i64().is_some());
    assert_eq!(r["used_from"], "lan");
    assert!(r.get("code").is_none() && r.get("code_hash").is_none());
    for who in [&m, &t.member("carol").await] {
        assert_eq!(
            t.call(get("/api/resets", who)).await.0,
            StatusCode::FORBIDDEN
        );
    }
}

/// After a reset the person is told once who reset it.
#[tokio::test]
async fn the_member_is_told_once() {
    let t = TestServer::new().await;
    let _owner = t.owner().await;
    let m = moderator(&t, "mod").await;
    let bob = t.member("bob").await;
    let code = make_link(&t, &m, &bob.user.user_id).await.1.unwrap();
    assert_eq!(
        reset(&t, &code, NEW_PASSWORD).await.0,
        StatusCode::NO_CONTENT
    );
    let body = json!({ "username": "bob", "password": NEW_PASSWORD });
    let (s, h, _) = t
        .call(request(Method::POST, "/api/login", Some(&body), None))
        .await;
    assert_eq!(s, StatusCode::OK);
    let cookie = common::set_cookie(&h);
    let take = || request(Method::POST, "/api/me/reset-notices", None, Some(&cookie));
    let (s, _, v) = t.call(take()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v[0]["by"], "mod");
    assert!(v[0]["at"].as_i64().is_some());
    assert_eq!(t.call(take()).await.2, json!([]));
}

/// A link a moderator made for a member dies when that member becomes a moderator, is banned, or
/// changes their own password.
#[tokio::test]
async fn role_change_ban_and_password_change_end_a_live_link() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let m = moderator(&t, "mod").await;
    let bob = t.member("bob").await;
    let id = bob.user.user_id.clone();
    let code = make_link(&t, &m, &id).await.1.unwrap();
    let role = json!({ "role": "moderator" });
    let (s, _, _) = t
        .call(request(
            Method::PATCH,
            &format!("/api/users/{id}/role"),
            Some(&role),
            Some(&owner.cookie),
        ))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        reset(&t, &code, NEW_PASSWORD).await.0,
        StatusCode::NOT_FOUND
    );

    let code = make_link(&t, &owner, &id).await.1.unwrap();
    let (s, _, _) = t
        .call(post(&format!("/api/users/{id}/ban"), json!({}), &owner))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, _) = t
        .call(post(&format!("/api/users/{id}/unban"), json!({}), &owner))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        reset(&t, &code, NEW_PASSWORD).await.0,
        StatusCode::NOT_FOUND
    );

    let code = make_link(&t, &owner, &id).await.1.unwrap();
    let bob = t.login("bob").await;
    let change = json!({ "old": PASSWORD, "new": "test-password-3" });
    assert_eq!(
        t.call(post("/api/me/password", change, &bob)).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        reset(&t, &code, NEW_PASSWORD).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        login_status(&t, "bob", "test-password-3").await,
        StatusCode::OK
    );
}

/// Banning or demoting a moderator ends the live links they made, even ones nobody used yet.
#[tokio::test]
async fn banning_or_demoting_the_maker_ends_their_links() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let m = moderator(&t, "mod").await;
    let bob = t.member("bob").await;
    let carol = t.member("carol").await;
    let mid = m.user.user_id.clone();
    let for_bob = make_link(&t, &m, &bob.user.user_id).await.1.unwrap();
    let by_owner = make_link(&t, &owner, &carol.user.user_id).await.1.unwrap();
    let (s, _, _) = t
        .call(post(&format!("/api/users/{mid}/ban"), json!({}), &owner))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        reset(&t, &for_bob, NEW_PASSWORD).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(login_status(&t, "bob", PASSWORD).await, StatusCode::OK);
    // Another maker's link is untouched.
    assert_eq!(
        reset(&t, &by_owner, NEW_PASSWORD).await.0,
        StatusCode::NO_CONTENT
    );

    let (s, _, _) = t
        .call(post(&format!("/api/users/{mid}/unban"), json!({}), &owner))
        .await;
    assert_eq!(s, StatusCode::OK);
    let m = t.login("mod").await;
    let for_bob = make_link(&t, &m, &bob.user.user_id).await.1.unwrap();
    let role = json!({ "role": "member" });
    let (s, _, _) = t
        .call(request(
            Method::PATCH,
            &format!("/api/users/{mid}/role"),
            Some(&role),
            Some(&owner.cookie),
        ))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        reset(&t, &for_bob, NEW_PASSWORD).await.0,
        StatusCode::NOT_FOUND
    );
}

/// After a takeover the taker sees the notice first; when the member gets back in with the
/// owner's link, they see both resets.
#[tokio::test]
async fn the_member_hears_about_a_takeover_after_recovery() {
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let m = moderator(&t, "mod").await;
    let bob = t.member("bob").await;
    let notices = async |password: &str| {
        let body = json!({ "username": "bob", "password": password });
        let (s, h, _) = t
            .call(request(Method::POST, "/api/login", Some(&body), None))
            .await;
        assert_eq!(s, StatusCode::OK);
        let cookie = common::set_cookie(&h);
        t.call(request(
            Method::POST,
            "/api/me/reset-notices",
            None,
            Some(&cookie),
        ))
        .await
        .2
    };
    let code = make_link(&t, &m, &bob.user.user_id).await.1.unwrap();
    assert_eq!(
        reset(&t, &code, "test-password-7").await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        notices("test-password-7").await.as_array().unwrap().len(),
        1
    );
    // The member can not sign in; the owner gives them a link.
    let code = make_link(&t, &owner, &bob.user.user_id).await.1.unwrap();
    assert_eq!(
        reset(&t, &code, NEW_PASSWORD).await.0,
        StatusCode::NO_CONTENT
    );
    let v = notices(NEW_PASSWORD).await;
    let by: Vec<_> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["by"].clone())
        .collect();
    assert_eq!(by, [json!("mod"), json!("owner")], "{v}");
    // The owner's own recovery link does not hide the moderator's reset from the owner.
    let (_, _, v) = t.call(get("/api/resets", &owner)).await;
    let makers: Vec<_> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["created_by"].clone())
        .collect();
    assert_eq!(
        makers,
        [json!(owner.user.user_id), json!(m.user.user_id)],
        "{v}"
    );
}
