//! The room routes for every role: who sees what, and that only the owner changes rooms.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{ORIGIN, Session, TestServer, get, post, request};
use serde_json::{Value, json};

async fn call(
    t: &TestServer,
    method: Method,
    path: &str,
    body: Value,
    s: &Session,
) -> (StatusCode, Value) {
    let (status, _, v) = t
        .call(request(method, path, Some(&body), Some(&s.cookie)))
        .await;
    (status, v)
}

async fn room_names(t: &TestServer, s: &Session) -> Vec<String> {
    let (status, _, v) = t.call(get("/api/rooms", s)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect()
}

async fn moderator(t: &TestServer, name: &str) -> Session {
    let code = t.invite(&["invite", "--role", "moderator"]).await;
    t.signup(&code, name).await
}

/// The owner makes `secret`, restricted to `listed`. Returns its id.
async fn secret_room(t: &TestServer, o: &Session, listed: &Session) -> i64 {
    let (s, _, v) = t
        .call(post("/api/rooms", json!({ "name": "secret" }), o))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_i64().unwrap();
    let path = format!("/api/rooms/{id}");
    let (s, v) = call(t, Method::PATCH, &path, json!({ "restricted": true }), o).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let ids = json!({ "user_ids": [listed.user.user_id] });
    let (s, v) = call(t, Method::PUT, &format!("{path}/members"), ids, o).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["members"], json!([listed.user.user_id]));
    id
}

#[tokio::test]
async fn members_do_not_see_restricted_rooms() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let alice = t.member("alice").await;
    let bob = t.member("bob").await;
    secret_room(&t, &o, &alice).await;
    assert_eq!(room_names(&t, &bob).await, ["general", "hangout"]);
    assert_eq!(
        room_names(&t, &alice).await,
        ["general", "hangout", "secret"]
    );
    let (s, _, _) = t.call(request(Method::GET, "/api/rooms", None, None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn moderator_sees_restricted_rooms() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let md = moderator(&t, "mod").await;
    let alice = t.member("alice").await;
    secret_room(&t, &o, &alice).await;
    assert_eq!(room_names(&t, &md).await, ["general", "hangout", "secret"]);
    assert_eq!(room_names(&t, &o).await, ["general", "hangout", "secret"]);
}

#[tokio::test]
async fn room_admin_is_owner_only() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let md = moderator(&t, "mod").await;
    let m = t.member("alice").await;
    // The target room is restricted with a known list, so a refused write to it would show.
    let carol = t.member("carol").await;
    let (_, _, v) = t.call(get("/api/rooms", &o)).await;
    let g = v[0]["id"].as_i64().unwrap();
    let path = format!("/api/rooms/{g}");
    let (s, v) = call(&t, Method::PATCH, &path, json!({ "restricted": true }), &o).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let list = json!({ "user_ids": [carol.user.user_id] });
    let (s, v) = call(&t, Method::PUT, &format!("{path}/members"), list, &o).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, _, v) = t.call(get("/api/rooms", &o)).await;
    let before = v.clone();
    assert_eq!(v[0]["members"], json!([carol.user.user_id]));
    let ids: Vec<i64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_i64().unwrap())
        .collect();
    let writes = [
        (
            Method::POST,
            "/api/rooms".to_string(),
            json!({ "name": "new" }),
        ),
        (
            Method::PATCH,
            format!("/api/rooms/{g}"),
            json!({ "name": "x", "restricted": true }),
        ),
        (
            Method::PUT,
            "/api/rooms/order".to_string(),
            json!({ "ids": [ids[1], ids[0]] }),
        ),
        (
            Method::PUT,
            format!("/api/rooms/{g}/members"),
            json!({ "user_ids": [m.user.user_id] }),
        ),
    ];
    for who in [&md, &m] {
        for (method, path, body) in &writes {
            let (s, v) = call(&t, method.clone(), path, body.clone(), who).await;
            assert_eq!(
                s,
                StatusCode::FORBIDDEN,
                "{} {method} {path} {v}",
                who.user.username
            );
            let (_, _, after) = t.call(get("/api/rooms", &o)).await;
            assert_eq!(
                after, before,
                "{method} {path} by {} wrote",
                who.user.username
            );
        }
    }

    // The owner can do all four.
    let ok = [
        StatusCode::CREATED,
        StatusCode::OK,
        StatusCode::CONFLICT,
        StatusCode::OK,
    ];
    for ((method, path, body), want) in writes.iter().zip(ok) {
        let (s, v) = call(&t, method.clone(), path, body.clone(), &o).await;
        assert_eq!(s, want, "{method} {path} {v}");
    }
    // The order must now name the new room too.
    let (_, _, v) = t.call(get("/api/rooms", &o)).await;
    let mut ids: Vec<i64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_i64().unwrap())
        .collect();
    ids.reverse();
    let (s, v) = call(
        &t,
        Method::PUT,
        "/api/rooms/order",
        json!({ "ids": ids }),
        &o,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(room_names(&t, &o).await, ["new", "hangout", "x"]);
    // The owner's list replaced carol's with alice: only alice sees the restricted room.
    assert_eq!(room_names(&t, &m).await, ["new", "hangout", "x"]);
    assert_eq!(room_names(&t, &carol).await, ["new", "hangout"]);
    let bob = t.member("bob").await;
    assert_eq!(room_names(&t, &bob).await, ["new", "hangout"]);

    // Unknown rooms and accounts are 404.
    let (s, _) = call(
        &t,
        Method::PATCH,
        "/api/rooms/999",
        json!({ "name": "y" }),
        &o,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let path = format!("/api/rooms/{g}/members");
    let body = json!({ "user_ids": ["f".repeat(32)] });
    let (s, _) = call(&t, Method::PUT, &path, body, &o).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn order_rejects_duplicate_and_unknown_ids() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let (_, _, v) = t.call(get("/api/rooms", &o)).await;
    let (a, b) = (v[0]["id"].as_i64().unwrap(), v[1]["id"].as_i64().unwrap());
    for ids in [
        json!([a, a]),
        json!([a, b, b]),
        json!([a, 999]),
        json!([a, b, 999]),
    ] {
        let (s, v) = call(
            &t,
            Method::PUT,
            "/api/rooms/order",
            json!({ "ids": ids }),
            &o,
        )
        .await;
        assert_eq!(s, StatusCode::CONFLICT, "{ids} {v}");
    }
    assert_eq!(room_names(&t, &o).await, ["general", "hangout"]);
}

#[tokio::test]
async fn opening_a_room_clears_its_member_list() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let alice = t.member("alice").await;
    let bob = t.member("bob").await;
    let id = secret_room(&t, &o, &alice).await;
    let path = format!("/api/rooms/{id}");
    for restricted in [false, true] {
        let (s, v) = call(
            &t,
            Method::PATCH,
            &path,
            json!({ "restricted": restricted }),
            &o,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    let (_, _, v) = t.call(get("/api/rooms", &o)).await;
    assert_eq!(v[2]["members"], json!([]), "{v}");
    assert_eq!(room_names(&t, &alice).await, ["general", "hangout"]);
    assert_eq!(room_names(&t, &bob).await, ["general", "hangout"]);
}

#[tokio::test]
async fn banned_account_can_not_be_listed() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let alice = t.member("alice").await;
    let id = secret_room(&t, &o, &alice).await;
    let bob = t.member("bob").await;
    t.state
        .store
        .set_banned(&bob.user.user_id, true, 1)
        .await
        .unwrap();
    let body = json!({ "user_ids": [alice.user.user_id, bob.user.user_id] });
    let (s, v) = call(
        &t,
        Method::PUT,
        &format!("/api/rooms/{id}/members"),
        body,
        &o,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (_, _, v) = t.call(get("/api/rooms", &o)).await;
    assert_eq!(v[2]["members"], json!([alice.user.user_id]));
}

#[tokio::test]
async fn racing_room_edits_do_not_fail() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let alice = t.member("alice").await;
    let id = secret_room(&t, &o, &alice).await;
    let (_, _, v) = t.call(get("/api/rooms", &o)).await;
    let ids: Vec<i64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_i64().unwrap())
        .collect();
    let rev: Vec<i64> = ids.iter().rev().copied().collect();
    let members = format!("/api/rooms/{id}/members");
    let t = &t;
    let o = &o;
    let calls = (0..8).map(|i| {
        let (ids, rev, members) = (ids.clone(), rev.clone(), members.clone());
        async move {
            if i % 2 == 0 {
                let order = if i % 4 == 0 { ids } else { rev };
                call(
                    t,
                    Method::PUT,
                    "/api/rooms/order",
                    json!({ "ids": order }),
                    o,
                )
                .await
            } else {
                call(t, Method::PUT, &members, json!({ "user_ids": [] }), o).await
            }
        }
    });
    for (s, v) in futures_util::future::join_all(calls).await {
        assert!(
            s == StatusCode::OK || s == StatusCode::NO_CONTENT,
            "{s} {v}"
        );
    }
}

#[tokio::test]
async fn room_name_rules() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let g = {
        let (_, _, v) = t.call(get("/api/rooms", &o)).await;
        v[0]["id"].as_i64().unwrap()
    };
    let bad = [
        "",
        "   ",
        &"a".repeat(33),
        "\u{202E}lareneg",
        "a\u{200B}b",
        "a\u{7}b",
    ];
    for name in bad {
        let (s, _, v) = t
            .call(post("/api/rooms", json!({ "name": name }), &o))
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{name:?} {v}");
        let path = format!("/api/rooms/{g}");
        let (s, v) = call(&t, Method::PATCH, &path, json!({ "name": name }), &o).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{name:?} {v}");
    }
    assert_eq!(room_names(&t, &o).await, ["general", "hangout"]);
    // 32 chars pass, whitespace is collapsed, and a case-only duplicate is 409.
    let (s, _, v) = t
        .call(post("/api/rooms", json!({ "name": "a".repeat(32) }), &o))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, _, v) = t
        .call(post(
            "/api/rooms",
            json!({ "name": "  movie   night " }),
            &o,
        ))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["name"], "movie night");
    let (s, _, _) = t
        .call(post("/api/rooms", json!({ "name": "General" }), &o))
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
}

/// Archive: hidden from every list but the owner's (marked), closed for history, posts and
/// search, owner only, never the last room; unarchive brings it back with its messages.
#[tokio::test]
async fn archived_rooms_are_hidden_and_closed() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let md = moderator(&t, "mod").await;
    let m = t.member("alice").await;
    let (_, _, v) = t.call(get("/api/rooms", &o)).await;
    let (general, hangout) = (v[0]["id"].as_i64().unwrap(), v[1]["id"].as_i64().unwrap());
    let msgs = format!("/api/rooms/{hangout}/messages");
    let body = json!({ "body": "needle kept", "reply_to": null, "attachments": [] });
    let (s, _, v) = t.call(post(&msgs, body.clone(), &m)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let up = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/rooms/{hangout}/uploads?name=f.bin"))
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, &m.cookie)
        .body(Body::from(b"kept".to_vec()))
        .unwrap();
    let (s, _, v) = t.call(up).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let file = format!("/api/uploads/{}", v["attachment"]["id"].as_str().unwrap());
    let path = format!("/api/rooms/{hangout}");
    let archive = |on: bool| json!({ "archived": on });

    for who in [&md, &m] {
        let (s, _) = call(&t, Method::PATCH, &path, archive(true), who).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{}", who.user.username);
    }
    let (s, v) = call(&t, Method::PATCH, &path, archive(true), &o).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["archived"], true);

    for who in [&md, &m] {
        assert_eq!(
            room_names(&t, who).await,
            ["general"],
            "{}",
            who.user.username
        );
    }
    let (_, _, v) = t.call(get("/api/rooms", &o)).await;
    assert_eq!(v[1]["name"], "hangout");
    assert_eq!(
        (v[0]["archived"].clone(), v[1]["archived"].clone()),
        (json!(false), json!(true))
    );
    for who in [&o, &md, &m] {
        let (s, _, _) = t.call(get(&msgs, who)).await;
        assert_eq!(
            s,
            StatusCode::NOT_FOUND,
            "history for {}",
            who.user.username
        );
        let (s, _, _) = t.call(post(&msgs, body.clone(), who)).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "post for {}", who.user.username);
        let (s, _, v) = t.call(get("/api/search?q=needle", who)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["hits"], json!([]), "search for {}", who.user.username);
        // Files are kept on disk but not served while archived, the uploader's too.
        let (s, _, _) = t.call(get(&file, who)).await;
        assert_eq!(
            s,
            StatusCode::NOT_FOUND,
            "download for {}",
            who.user.username
        );
    }

    // general is the last room left open.
    let last = format!("/api/rooms/{general}");
    let (s, v) = call(&t, Method::PATCH, &last, archive(true), &o).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(room_names(&t, &m).await, ["general"]);

    let (s, v) = call(&t, Method::PATCH, &path, archive(false), &o).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["archived"], false);
    assert_eq!(room_names(&t, &m).await, ["general", "hangout"]);
    let (s, _, v) = t.call(get(&msgs, &m)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["messages"][0]["body"], "needle kept");
    let (s, _, _) = t.call(get(&file, &m)).await;
    assert_eq!(s, StatusCode::OK);
    let (_, _, v) = t.call(get("/api/search?q=needle", &m)).await;
    assert_eq!(v["hits"].as_array().unwrap().len(), 1);
    // A default room may be archived while another stays open.
    let (s, v) = call(&t, Method::PATCH, &last, archive(true), &o).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}
