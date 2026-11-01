//! The admin routes: users, invites, roles and bans, for every role of caller and target.

mod common;

use axum::http::{Method, StatusCode};
use common::{ORIGIN, PASSWORD, Session, TestServer, get, post, request};
use serde_json::{Value, json};
use vesper_protocol::Role;

fn invite_body(role: &str) -> Value {
    json!({ "role": role, "max_uses": 1, "expires_hours": 24 })
}

async fn status(
    t: &TestServer,
    method: Method,
    path: &str,
    body: Option<Value>,
    s: Option<&Session>,
) -> StatusCode {
    let req = request(method, path, body.as_ref(), s.map(|s| s.cookie.as_str()));
    t.call(req).await.0
}

async fn login_status(t: &TestServer, username: &str) -> StatusCode {
    let body = json!({ "username": username, "password": PASSWORD });
    status(t, Method::POST, "/api/login", Some(body), None).await
}

/// A moderator made through a CLI moderator invite.
async fn moderator(t: &TestServer, name: &str) -> Session {
    let code = t.invite(&["invite", "--role", "moderator"]).await;
    t.signup(&code, name).await
}

#[tokio::test]
async fn member_cannot_create_invites() {
    let t = TestServer::new().await;
    let m = t.member("alice").await;
    let (s, _, _) = t
        .call(post("/api/invites", invite_body("member"), &m))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn moderator_creates_member_invite_not_moderator_invite() {
    let t = TestServer::new().await;
    let m = moderator(&t, "mod").await;
    let (s, _, v) = t
        .call(post("/api/invites", invite_body("member"), &m))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let url = v["url"].as_str().unwrap();
    assert!(url.starts_with(&format!("{ORIGIN}/invite#")), "{url}");
    assert_eq!(v["info"]["role"], "member");
    assert_eq!(v["info"]["created_by"], m.user.user_id.as_str());
    // The link works for a signup.
    t.signup(url.split_once('#').unwrap().1, "bob").await;
    let (s, _, _) = t
        .call(post("/api/invites", invite_body("moderator"), &m))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn invite_limits_are_checked() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    for body in [
        json!({ "role": "member", "max_uses": 0, "expires_hours": 24 }),
        json!({ "role": "member", "max_uses": 51, "expires_hours": 24 }),
        json!({ "role": "member", "max_uses": 1, "expires_hours": 0 }),
        json!({ "role": "member", "max_uses": 1, "expires_hours": 721 }),
        json!({ "role": "member", "max_uses": 1 }),
    ] {
        let (s, _, v) = t.call(post("/api/invites", body.clone(), &o)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body} {v}");
    }
    let body = json!({ "role": "moderator", "max_uses": 50, "expires_hours": 720 });
    let (s, _, v) = t.call(post("/api/invites", body, &o)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["info"]["max_uses"], 50);
}

#[tokio::test]
async fn owner_role_cannot_be_minted_or_assigned() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let m = t.member("alice").await;
    let (s, _, _) = t.call(post("/api/invites", invite_body("owner"), &o)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let path = format!("/api/users/{}/role", m.user.user_id);
    let s = status(
        &t,
        Method::PATCH,
        &path,
        Some(json!({ "role": "owner" })),
        Some(&o),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(
        t.state.store.user(&m.user.user_id).await.unwrap().role,
        Role::Member
    );
}

#[tokio::test]
async fn role_change_and_ban_are_owner_only() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let md = moderator(&t, "mod").await;
    let m = t.member("alice").await;
    let id = &m.user.user_id;
    let role = format!("/api/users/{id}/role");
    let ban = format!("/api/users/{id}/ban");
    let unban = format!("/api/users/{id}/unban");

    let to_mod = || Some(json!({ "role": "moderator" }));
    assert_eq!(
        status(&t, Method::PATCH, &role, to_mod(), Some(&md)).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(&t, Method::POST, &ban, None, Some(&md)).await,
        StatusCode::FORBIDDEN
    );

    let (s, _, v) = t
        .call(request(
            Method::PATCH,
            &role,
            to_mod().as_ref(),
            Some(&o.cookie),
        ))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["role"], "moderator");
    // The new role counts on the user's next request, with the same cookie.
    let (s, _, _) = t.call(get("/api/invites", &m)).await;
    assert_eq!(s, StatusCode::OK);

    let (s, _, v) = t.call(post(&ban, Value::Null, &o)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["banned"], true);
    assert_eq!(t.call(get("/api/me", &m)).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(login_status(&t, "alice").await, StatusCode::UNAUTHORIZED);

    let (s, _, v) = t.call(post(&unban, Value::Null, &o)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["banned"], false);
    let again = t.login("alice").await;
    assert_eq!(t.call(get("/api/me", &again)).await.0, StatusCode::OK);
    // The old cookie stays dead: the ban deleted its session.
    assert_eq!(t.call(get("/api/me", &m)).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn ban_ends_every_session_of_the_user() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let first = t.member("alice").await;
    let second = t.login("alice").await;
    for s in [&first, &second] {
        assert_eq!(t.call(get("/api/me", s)).await.0, StatusCode::OK);
    }
    let ban = format!("/api/users/{}/ban", first.user.user_id);
    assert_eq!(
        status(&t, Method::POST, &ban, None, Some(&o)).await,
        StatusCode::OK
    );
    for s in [&first, &second] {
        assert_eq!(t.call(get("/api/me", s)).await.0, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(login_status(&t, "alice").await, StatusCode::UNAUTHORIZED);
    let unban = format!("/api/users/{}/unban", first.user.user_id);
    assert_eq!(
        status(&t, Method::POST, &unban, None, Some(&o)).await,
        StatusCode::OK
    );
    let third = t.login("alice").await;
    assert_eq!(t.call(get("/api/me", &third)).await.0, StatusCode::OK);
    for s in [&first, &second] {
        assert_eq!(t.call(get("/api/me", s)).await.0, StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn owner_cannot_ban_self() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let path = format!("/api/users/{}/ban", o.user.user_id);
    assert_eq!(
        status(&t, Method::POST, &path, None, Some(&o)).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(t.call(get("/api/me", &o)).await.0, StatusCode::OK);
}

#[tokio::test]
async fn revoked_invite_preview_is_404() {
    let t = TestServer::new().await;
    let md = moderator(&t, "mod").await;
    let (_, _, v) = t
        .call(post("/api/invites", invite_body("member"), &md))
        .await;
    let code = v["url"]
        .as_str()
        .unwrap()
        .split_once('#')
        .unwrap()
        .1
        .to_string();
    let id = v["info"]["id"].as_i64().unwrap();
    let preview = format!("/api/invites/preview?code={code}");
    assert_eq!(
        status(&t, Method::GET, &preview, None, None).await,
        StatusCode::OK
    );
    let path = format!("/api/invites/{id}");
    assert_eq!(
        status(&t, Method::DELETE, &path, None, Some(&md)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        status(&t, Method::GET, &preview, None, None).await,
        StatusCode::NOT_FOUND
    );
    let (_, _, list) = t.call(get("/api/invites", &md)).await;
    assert_eq!(list[0]["revoked"], true);
    assert_eq!(
        status(&t, Method::DELETE, "/api/invites/999", None, Some(&md)).await,
        StatusCode::NOT_FOUND
    );
}

/// Creates an invite as `by` and returns its id and code.
async fn new_invite(t: &TestServer, by: &Session, role: &str) -> (i64, String) {
    let (s, _, v) = t.call(post("/api/invites", invite_body(role), by)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let code = v["url"].as_str().unwrap().split_once('#').unwrap().1;
    (v["info"]["id"].as_i64().unwrap(), code.to_string())
}

#[tokio::test]
async fn moderator_revokes_member_invites_only() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let md = moderator(&t, "mod").await;
    let del = |id: i64| format!("/api/invites/{id}");

    let (member_inv, _) = new_invite(&t, &o, "member").await;
    assert_eq!(
        status(&t, Method::DELETE, &del(member_inv), None, Some(&md)).await,
        StatusCode::NO_CONTENT
    );

    let (mod_inv, code) = new_invite(&t, &o, "moderator").await;
    let (s, _, v) = t
        .call(request(
            Method::DELETE,
            &del(mod_inv),
            None,
            Some(&md.cookie),
        ))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(v["code"], "forbidden", "{v}");
    // Still valid: it signs up a new moderator.
    let bob = t.signup(&code, "bob").await;
    assert_eq!(bob.user.role, Role::Moderator);

    // The owner revokes either kind.
    for role in ["member", "moderator"] {
        let (id, _) = new_invite(&t, &o, role).await;
        assert_eq!(
            status(&t, Method::DELETE, &del(id), None, Some(&o)).await,
            StatusCode::NO_CONTENT,
            "{role}"
        );
    }
    // Unknown ids stay 404 for a moderator.
    assert_eq!(
        status(&t, Method::DELETE, &del(999), None, Some(&md)).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn unknown_user_is_404() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let path = format!("/api/users/{}/ban", "a".repeat(32));
    assert_eq!(
        status(&t, Method::POST, &path, None, Some(&o)).await,
        StatusCode::NOT_FOUND
    );
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Who {
    Out,
    Member,
    Mod,
    Owner,
}

#[derive(Clone, Copy, Debug)]
enum Target {
    Member,
    Mod,
    Owner,
    /// The caller's own account.
    Me,
}

/// Every route, every caller role, and for the user routes every target role.
#[tokio::test]
async fn every_route_for_every_role() {
    use StatusCode as S;
    let t = TestServer::new().await;
    let owner = t.owner().await;
    let md = moderator(&t, "mod").await;
    let mut md2 = moderator(&t, "mod2").await;
    let mem = t.member("mem").await;
    let mut mem2 = t.member("mem2").await;
    let n_invites = || async { t.state.store.invites().await.unwrap().len() };
    let callers = [
        (Who::Out, None),
        (Who::Member, Some(&mem)),
        (Who::Mod, Some(&md)),
        (Who::Owner, Some(&owner)),
    ];
    let pick = |w: Who, ok_mod: S, ok_owner: S, member: S| match w {
        Who::Out => S::UNAUTHORIZED,
        Who::Member => member,
        Who::Mod => ok_mod,
        Who::Owner => ok_owner,
    };

    for (who, s) in callers {
        let got = status(&t, Method::GET, "/api/users", None, s).await;
        assert_eq!(got, pick(who, S::OK, S::OK, S::OK), "users {who:?}");
        let got = status(&t, Method::GET, "/api/invites", None, s).await;
        assert_eq!(
            got,
            pick(who, S::OK, S::OK, S::FORBIDDEN),
            "invites {who:?}"
        );
        for (role, want_mod, want_owner) in [
            ("member", S::CREATED, S::CREATED),
            ("moderator", S::FORBIDDEN, S::CREATED),
            ("owner", S::BAD_REQUEST, S::BAD_REQUEST),
        ] {
            let before = n_invites().await;
            let got = status(&t, Method::POST, "/api/invites", Some(invite_body(role)), s).await;
            let want = pick(who, want_mod, want_owner, S::FORBIDDEN);
            assert_eq!(got, want, "create {role} invite {who:?}");
            let grew = if want == S::CREATED { 1 } else { 0 };
            assert_eq!(n_invites().await, before + grew, "create {role} {who:?}");
        }
        // A fresh invite, made by the owner, for each revoke. Moderators revoke member invites only.
        for (role, want_mod) in [("member", S::NO_CONTENT), ("moderator", S::FORBIDDEN)] {
            let (_, _, v) = t
                .call(post("/api/invites", invite_body(role), &owner))
                .await;
            let inv_id = v["info"]["id"].as_i64().unwrap();
            let path = format!("/api/invites/{inv_id}");
            let got = status(&t, Method::DELETE, &path, None, s).await;
            let want = pick(who, want_mod, S::NO_CONTENT, S::FORBIDDEN);
            assert_eq!(got, want, "revoke {role} {who:?}");
            let invites = t.state.store.invites().await.unwrap();
            let revoked = invites.iter().find(|i| i.id == inv_id).unwrap().revoked;
            assert_eq!(
                revoked,
                want == S::NO_CONTENT,
                "revoked flag {role} {who:?}"
            );
        }

        for target in [Target::Member, Target::Mod, Target::Owner, Target::Me] {
            let target_session = match target {
                Target::Member => &mem2,
                Target::Mod => &md2,
                Target::Owner => &owner,
                Target::Me => s.unwrap_or(&mem2),
            };
            let tu = target_session.user.clone();
            let cookie = target_session.cookie.clone();
            let id = &tu.user_id;
            // Owner on a member or moderator: allowed. On an owner or themselves: 400.
            let owner_want = match target {
                Target::Member | Target::Mod => S::OK,
                Target::Owner | Target::Me => S::BAD_REQUEST,
            };
            let want = pick(who, S::FORBIDDEN, owner_want, S::FORBIDDEN);
            let new_role = if tu.role == Role::Member {
                "moderator"
            } else {
                "member"
            };
            for (method, path, body) in [
                (
                    Method::PATCH,
                    format!("/api/users/{id}/role"),
                    Some(json!({ "role": new_role })),
                ),
                (
                    Method::PATCH,
                    format!("/api/users/{id}/role"),
                    Some(json!({ "role": "owner" })),
                ),
                (Method::POST, format!("/api/users/{id}/ban"), None),
                (Method::POST, format!("/api/users/{id}/unban"), None),
            ] {
                let got = status(&t, method.clone(), &path, body.clone(), s).await;
                let want = match (&body, who) {
                    // Promoting to owner is refused even where the target is fair game.
                    (Some(b), Who::Owner) if b["role"] == "owner" && want == S::OK => {
                        S::BAD_REQUEST
                    }
                    _ => want,
                };
                assert_eq!(
                    got, want,
                    "{method} {path} {body:?} by {who:?} on {target:?}"
                );
                let after = t.state.store.user(id).await.unwrap();
                if got.is_success() {
                    // Undo the change so the next case starts from the same state.
                    t.state.store.set_role(id, tu.role).await.unwrap();
                    t.state.store.set_banned(id, false, 0).await.unwrap();
                    // A ban ended the target's sessions: log it in again.
                    if path.ends_with("/ban") && got == S::OK {
                        let name = &tu.username;
                        match target {
                            Target::Member => mem2 = t.login(name).await,
                            Target::Mod => md2 = t.login(name).await,
                            _ => unreachable!("only members and mods can be banned"),
                        }
                    }
                } else {
                    // A refusal changes nothing: same role, same ban flag, session still live.
                    assert_eq!(
                        after.role, tu.role,
                        "role changed: {method} {path} by {who:?} on {target:?}"
                    );
                    assert!(
                        !after.banned,
                        "banned: {method} {path} by {who:?} on {target:?}"
                    );
                    let req = request(Method::GET, "/api/me", None, Some(cookie.as_str()));
                    let c = t.call(req).await.0;
                    assert_eq!(
                        c,
                        S::OK,
                        "session lost: {method} {path} by {who:?} on {target:?}"
                    );
                }
            }
        }
    }
    // Nothing above changed who the owner is, and the callers' own sessions are still live.
    let users = t.state.store.users().await.unwrap();
    let owners: Vec<_> = users.iter().filter(|u| u.role == Role::Owner).collect();
    assert_eq!(owners.len(), 1);
    assert_eq!(owners[0].user_id, owner.user.user_id);
    for s in [&owner, &md, &mem] {
        assert_eq!(t.call(get("/api/me", s)).await.0, S::OK);
    }
}
