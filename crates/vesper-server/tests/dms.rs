//! Direct messages: a DM is a locked room of two. The owner and moderators never see one, no
//! room edit reaches one, and a banned account's DMs stay readable to the other person.

mod common;

use axum::http::{Method, StatusCode};
use common::{Session, TestServer, get, post, request};
use serde_json::{Value, json};

async fn open(t: &TestServer, from: &Session, to: &Session) -> (StatusCode, Value) {
    let (s, _, v) = t
        .call(post(
            "/api/dms",
            json!({ "user_id": to.user.user_id }),
            from,
        ))
        .await;
    (s, v)
}

async fn dm(t: &TestServer, from: &Session, to: &Session) -> i64 {
    let (s, v) = open(t, from, to).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["id"].as_i64().unwrap()
}

async fn status(
    t: &TestServer,
    m: Method,
    path: &str,
    body: Option<Value>,
    s: &Session,
) -> StatusCode {
    let (st, _, _) = t
        .call(request(m, path, body.as_ref(), Some(&s.cookie)))
        .await;
    st
}

async fn say(t: &TestServer, s: &Session, room: i64, text: &str) {
    let (st, _, v) = t
        .call(post(
            &format!("/api/rooms/{room}/messages"),
            json!({ "body": text }),
            s,
        ))
        .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
}

async fn room_ids(t: &TestServer, s: &Session) -> Vec<i64> {
    let (_, _, v) = t.call(get("/api/rooms", s)).await;
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_i64().unwrap())
        .collect()
}

async fn moderator(t: &TestServer, name: &str) -> Session {
    let code = t.invite(&["invite", "--role", "moderator"]).await;
    t.signup(&code, name).await
}

#[tokio::test]
async fn one_dm_per_pair_from_either_side() {
    let t = TestServer::new().await;
    let alice = t.member("alice").await;
    let bob = t.member("bob").await;
    let (s, v) = open(&t, &alice, &bob).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["kind"], "dm");
    assert_eq!(v["restricted"], true);
    let id = v["id"].as_i64().unwrap();
    assert_eq!(dm(&t, &alice, &bob).await, id);
    assert_eq!(dm(&t, &bob, &alice).await, id);
    let mut members: Vec<String> = serde_json::from_value(v["members"].clone()).unwrap();
    members.sort();
    let mut want = vec![alice.user.user_id.clone(), bob.user.user_id.clone()];
    want.sort();
    assert_eq!(members, want);
}

#[tokio::test]
async fn bad_targets_are_refused() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let alice = t.member("alice").await;
    let bob = t.member("bob").await;
    assert_eq!(open(&t, &alice, &alice).await.0, StatusCode::BAD_REQUEST);
    let nobody = json!({ "user_id": "0".repeat(32) });
    let (s, _, _) = t.call(post("/api/dms", nobody, &alice)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let ban = format!("/api/users/{}/ban", bob.user.user_id);
    assert_eq!(
        status(&t, Method::POST, &ban, None, &o).await,
        StatusCode::OK
    );
    assert_eq!(open(&t, &alice, &bob).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn only_the_two_see_a_dm_whatever_their_role() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let md = moderator(&t, "mod").await;
    let alice = t.member("alice").await;
    let bob = t.member("bob").await;
    let carol = t.member("carol").await;
    let id = dm(&t, &alice, &bob).await;
    say(&t, &alice, id, "the secret needle").await;

    for who in [&alice, &bob] {
        assert!(room_ids(&t, who).await.contains(&id));
    }
    let history = format!("/api/rooms/{id}/messages");
    for who in [&o, &md, &carol] {
        assert!(!room_ids(&t, who).await.contains(&id));
        assert_eq!(
            status(&t, Method::GET, &history, None, who).await,
            StatusCode::NOT_FOUND
        );
        let (st, _, v) = t.call(post(&history, json!({ "body": "hi" }), who)).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{v}");
        let (st, _, v) = t.call(get("/api/search?q=needle", who)).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert!(v["hits"].as_array().unwrap().is_empty(), "{v}");
    }
    let (_, _, v) = t.call(get("/api/search?q=needle", &bob)).await;
    assert_eq!(v["hits"].as_array().unwrap().len(), 1, "{v}");
}

#[tokio::test]
async fn no_room_edit_reaches_a_dm() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let alice = t.member("alice").await;
    let bob = t.member("bob").await;
    let carol = t.member("carol").await;
    let id = dm(&t, &alice, &bob).await;
    let path = format!("/api/rooms/{id}");
    let edits = [
        (Method::PATCH, path.clone(), json!({ "restricted": false })),
        (Method::PATCH, path.clone(), json!({ "archived": true })),
        (Method::PATCH, path.clone(), json!({ "name": "renamed" })),
        (
            Method::PUT,
            format!("{path}/members"),
            json!({ "user_ids": [carol.user.user_id] }),
        ),
    ];
    for (m, p, body) in edits {
        assert_eq!(
            status(&t, m.clone(), &p, Some(body), &o).await,
            StatusCode::NOT_FOUND,
            "{m} {p}"
        );
    }
    // The members are unchanged and the DM still works.
    say(&t, &alice, id, "still here").await;
    assert_ne!(dm(&t, &carol, &alice).await, id);

    // Reordering lists the real rooms only, with a DM in the database.
    let mut real = room_ids(&t, &o).await;
    real.reverse();
    assert_eq!(
        status(
            &t,
            Method::PUT,
            "/api/rooms/order",
            Some(json!({ "ids": real })),
            &o
        )
        .await,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn a_dm_does_not_count_as_a_room_for_the_last_room_rule() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let alice = t.member("alice").await;
    let bob = t.member("bob").await;
    dm(&t, &alice, &bob).await;
    let rooms = room_ids(&t, &o).await;
    assert_eq!(rooms.len(), 2);
    // Archive one, then the other must be refused although a DM is not archived.
    let first = format!("/api/rooms/{}", rooms[0]);
    let second = format!("/api/rooms/{}", rooms[1]);
    let archive = || Some(json!({ "archived": true }));
    assert_eq!(
        status(&t, Method::PATCH, &first, archive(), &o).await,
        StatusCode::OK
    );
    assert_eq!(
        status(&t, Method::PATCH, &second, archive(), &o).await,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn a_banned_accounts_dm_stays_readable_to_the_other_person() {
    let t = TestServer::new().await;
    let o = t.owner().await;
    let alice = t.member("alice").await;
    let bob = t.member("bob").await;
    let id = dm(&t, &alice, &bob).await;
    say(&t, &alice, id, "before the ban").await;
    let ban = format!("/api/users/{}/ban", alice.user.user_id);
    assert_eq!(
        status(&t, Method::POST, &ban, None, &o).await,
        StatusCode::OK
    );
    let (st, _, v) = t
        .call(get(&format!("/api/rooms/{id}/messages"), &bob))
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v.to_string().contains("before the ban"));
}
