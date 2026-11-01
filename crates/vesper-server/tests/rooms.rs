//! The room routes for every role: who sees what, and that only the owner changes rooms.

mod common;

use axum::http::{Method, StatusCode};
use common::{Session, TestServer, get, post, request};
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
    let (_, _, v) = t.call(get("/api/rooms", &o)).await;
    let before = v.clone();
    let ids: Vec<i64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_i64().unwrap())
        .collect();
    let g = ids[0];
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
        }
    }
    let (_, _, after) = t.call(get("/api/rooms", &o)).await;
    assert_eq!(after, before, "a refused write changed rooms");

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
    // The member list set above makes the restricted room visible to alice only.
    assert_eq!(room_names(&t, &m).await, ["new", "hangout", "x"]);
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
