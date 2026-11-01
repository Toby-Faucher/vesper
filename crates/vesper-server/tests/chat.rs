//! Chat over REST plus the pushed events on real sockets. To prove an event did NOT arrive, a
//! test sends a later event the socket must get and checks nothing forbidden came before it
//! (each socket's queue keeps order).

mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{ORIGIN, Session, TestServer, get, post, request};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMsg;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use vesper_protocol::{ClientMsg, Message, Role, ServerMsg, Unread};

const WAIT: Duration = Duration::from_secs(10);

/// One socket: `hello` kept, everything after it in `msgs`.
struct Sock {
    hello: ServerMsg,
    msgs: mpsc::UnboundedReceiver<ServerMsg>,
    out: mpsc::UnboundedSender<String>,
}

impl Sock {
    async fn open(server: &TestServer, s: &Session) -> Sock {
        let mut req = format!("ws://{}/ws", server.addr)
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("cookie", s.cookie.parse().unwrap());
        req.headers_mut().insert("origin", ORIGIN.parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        let (mut sink, mut stream) = ws.split();
        let (tx, mut msgs) = mpsc::unbounded_channel();
        let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    m = out_rx.recv() => match m {
                        Some(t) => if sink.send(WsMsg::text(t)).await.is_err() { return },
                        None => return,
                    },
                    m = stream.next() => match m {
                        Some(Ok(WsMsg::Text(t))) => {
                            let _ = tx.send(serde_json::from_str(&t).expect("valid ServerMsg"));
                        }
                        Some(Ok(WsMsg::Close(_))) | Some(Err(_)) | None => return,
                        Some(Ok(_)) => {}
                    },
                }
            }
        });
        let hello = tokio::time::timeout(WAIT, msgs.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(hello, ServerMsg::Hello { .. }), "{hello:?}");
        Sock { hello, msgs, out }
    }

    fn typing(&self, room_id: i64) {
        let m = serde_json::to_string(&ClientMsg::Typing { room_id }).unwrap();
        self.out.send(m).unwrap();
    }

    fn unread(&self, room_id: i64) -> Unread {
        let ServerMsg::Hello { unread, .. } = &self.hello else {
            unreachable!()
        };
        unread
            .iter()
            .find(|u| u.room_id == room_id)
            .unwrap()
            .clone()
    }

    /// Messages up to and including the first that matches `stop`.
    async fn until(&mut self, what: &str, stop: impl Fn(&ServerMsg) -> bool) -> Vec<ServerMsg> {
        let deadline = tokio::time::Instant::now() + WAIT;
        let mut seen = Vec::new();
        loop {
            let m = tokio::time::timeout_at(deadline, self.msgs.recv())
                .await
                .unwrap_or_else(|_| panic!("no {what} in time; got {seen:?}"))
                .unwrap_or_else(|| panic!("socket closed before {what}"));
            let done = stop(&m);
            seen.push(m);
            if done {
                return seen;
            }
        }
    }

    async fn until_new(&mut self, body: &str) -> Vec<ServerMsg> {
        self.until(
            body,
            |m| matches!(m, ServerMsg::MessageNew { message } if message.body == body),
        )
        .await
    }
}

async fn room(server: &TestServer, name: &str) -> i64 {
    let rooms = server.state.store.rooms().await.unwrap();
    rooms.iter().find(|r| r.name == name).unwrap().id
}

/// The owner makes a restricted room listing `members`; returns its id.
async fn restricted(server: &TestServer, owner: &Session, members: &[&Session]) -> i64 {
    let (status, _, v) = server
        .call(post("/api/rooms", json!({ "name": "secret" }), owner))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = v["id"].as_i64().unwrap();
    let ids: Vec<&str> = members.iter().map(|s| s.user.user_id.as_str()).collect();
    let (status, _, _) = put(
        server,
        &format!("/api/rooms/{id}/members"),
        json!({ "user_ids": ids }),
        owner,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = server
        .call(request(
            Method::PATCH,
            &format!("/api/rooms/{id}"),
            Some(&json!({ "restricted": true })),
            Some(&owner.cookie),
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    id
}

async fn put(
    server: &TestServer,
    path: &str,
    body: Value,
    s: &Session,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    server
        .call(request(Method::PUT, path, Some(&body), Some(&s.cookie)))
        .await
}

async fn send(server: &TestServer, room: i64, body: &str, s: &Session) -> (StatusCode, Value) {
    let (status, _, v) = server
        .call(post(
            &format!("/api/rooms/{room}/messages"),
            json!({ "body": body, "reply_to": null, "attachments": [] }),
            s,
        ))
        .await;
    (status, v)
}

/// Posts and returns the stored message; panics unless 201.
async fn sent(server: &TestServer, room: i64, body: &str, s: &Session) -> Message {
    let (status, v) = send(server, room, body, s).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    serde_json::from_value(v).unwrap()
}

async fn delete(server: &TestServer, id: i64, s: &Session) -> StatusCode {
    let path = format!("/api/messages/{id}");
    server
        .call(request(Method::DELETE, &path, None, Some(&s.cookie)))
        .await
        .0
}

async fn edit(server: &TestServer, id: i64, body: &str, s: &Session) -> (StatusCode, Value) {
    let path = format!("/api/messages/{id}");
    let (status, _, v) = server
        .call(request(
            Method::PATCH,
            &path,
            Some(&json!({ "body": body })),
            Some(&s.cookie),
        ))
        .await;
    (status, v)
}

async fn history(server: &TestServer, room: i64, s: &Session) -> Vec<Message> {
    let (status, _, v) = server
        .call(get(&format!("/api/rooms/{room}/messages"), s))
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    serde_json::from_value(v["messages"].clone()).unwrap()
}

fn is_updated(id: i64, deleted: bool) -> impl Fn(&ServerMsg) -> bool {
    move |m| matches!(m, ServerMsg::MessageUpdated { message } if message.id == id && message.deleted == deleted)
}

#[tokio::test]
async fn send_and_receive_live() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let general = room(&server, "general").await;
    let mut b = Sock::open(&server, &bob).await;
    let msg = sent(&server, general, "hello bob", &alice).await;
    assert_eq!(msg.author_id, alice.user.user_id);
    let seen = b.until_new("hello bob").await;
    assert_eq!(
        seen.last(),
        Some(&ServerMsg::MessageNew {
            message: msg.clone()
        })
    );
    assert_eq!(history(&server, general, &bob).await, vec![msg.clone()]);
    // A reply, and the cursors.
    let (status, _, v) = server
        .call(post(
            &format!("/api/rooms/{general}/messages"),
            json!({ "body": "hi", "reply_to": msg.id }),
            &bob,
        ))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["reply_to"]["id"], msg.id);
    let page = |q: String| {
        let server = &server;
        let bob = &bob;
        async move {
            server
                .call(get(&format!("/api/rooms/{general}/messages?{q}"), bob))
                .await
        }
    };
    let (status, _, v) = page(format!("after={}", msg.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["messages"].as_array().unwrap().len(), 1);
    assert_eq!(v["more_before"], true);
    let (status, _, v) = page(format!("around={}&limit=10", msg.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["messages"].as_array().unwrap().len(), 2);
    assert_eq!(
        page("before=1&after=1".into()).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(page("around=999999".into()).await.0, StatusCode::NOT_FOUND);
    // A send from another origin never runs.
    let mut req = post(
        &format!("/api/rooms/{general}/messages"),
        json!({ "body": "x" }),
        &alice,
    );
    req.headers_mut().remove(header::ORIGIN);
    assert_eq!(server.call(req).await.0, StatusCode::FORBIDDEN);
    assert_eq!(history(&server, general, &bob).await.len(), 2);
}

#[tokio::test]
async fn send_refuses_oversize_and_control_chars() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let general = room(&server, "general").await;
    assert_eq!(
        send(&server, general, &"a".repeat(4001), &alice).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(&server, general, "a\u{0000}b", &alice).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(&server, general, " \n\t ", &alice).await.0,
        StatusCode::BAD_REQUEST
    );
    let (status, v) = send(&server, general, &"x".repeat(40 * 1024), &alice).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{v}");
    assert_eq!(v["code"], "too_large");
    assert!(history(&server, general, &alice).await.is_empty());
    // Characters, not bytes: 4000 two-byte chars pass.
    let ok = sent(&server, general, &"é".repeat(4000), &alice).await;
    assert_eq!(ok.body.chars().count(), 4000);
    let ok = sent(&server, general, &"a".repeat(4000), &alice).await;
    assert_eq!(history(&server, general, &alice).await.last(), Some(&ok));
    // Edits follow the same rule.
    assert_eq!(
        edit(&server, ok.id, "a\u{0}", &alice).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(history(&server, general, &alice).await.last(), Some(&ok));
}

#[tokio::test]
async fn message_rate_limit() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let general = room(&server, "general").await;
    for i in 0..10 {
        sent(&server, general, &format!("m{i}"), &alice).await;
    }
    let (status, _, v) = server
        .call(post(
            &format!("/api/rooms/{general}/messages"),
            json!({ "body": "m10" }),
            &alice,
        ))
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{v}");
    assert_eq!(history(&server, general, &alice).await.len(), 10);
    // Per user: bob is not held back.
    sent(&server, general, "bob", &bob).await;
}

#[tokio::test]
async fn moderator_deletes_any_member_only_own() {
    let server = TestServer::new().await;
    let owner = server.account("owner", Role::Owner).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let moder = server.account("moder", Role::Moderator).await;
    let general = room(&server, "general").await;
    let mut a = Sock::open(&server, &alice).await;
    let mut b = Sock::open(&server, &bob).await;

    let m1 = sent(&server, general, "one", &alice).await;
    assert_eq!(delete(&server, m1.id, &bob).await, StatusCode::FORBIDDEN);
    assert!(!history(&server, general, &bob).await[0].deleted);
    assert_eq!(delete(&server, m1.id, &moder).await, StatusCode::NO_CONTENT);
    for s in [&mut a, &mut b] {
        let seen = s.until("deleted", is_updated(m1.id, true)).await;
        let ServerMsg::MessageUpdated { message } = seen.last().unwrap() else {
            unreachable!()
        };
        assert!(message.body.is_empty());
    }
    // A second delete, and edit, react and pin on a tombstone, are 404.
    assert_eq!(delete(&server, m1.id, &alice).await, StatusCode::NOT_FOUND);
    assert_eq!(
        edit(&server, m1.id, "back", &alice).await.0,
        StatusCode::NOT_FOUND
    );
    let pin = format!("/api/messages/{}/pin", m1.id);
    assert_eq!(
        put(&server, &pin, json!({ "on": true }), &alice).await.0,
        StatusCode::NOT_FOUND
    );
    let react = format!("/api/messages/{}/reaction", m1.id);
    assert_eq!(
        put(
            &server,
            &react,
            json!({ "emoji": "👍", "on": true }),
            &alice
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    let m2 = sent(&server, general, "two", &bob).await;
    assert_eq!(delete(&server, m2.id, &bob).await, StatusCode::NO_CONTENT);
    let m3 = sent(&server, general, "three", &moder).await;
    assert_eq!(delete(&server, m3.id, &bob).await, StatusCode::FORBIDDEN);
    assert_eq!(delete(&server, m3.id, &owner).await, StatusCode::NO_CONTENT);
    let m4 = sent(&server, general, "four", &owner).await;
    assert_eq!(delete(&server, m4.id, &moder).await, StatusCode::NO_CONTENT);
    b.until("m4 deleted", is_updated(m4.id, true)).await;
    let deleted: Vec<bool> = history(&server, general, &bob)
        .await
        .iter()
        .map(|m| m.deleted)
        .collect();
    assert_eq!(deleted, [true, true, true, true]);
}

#[tokio::test]
async fn members_edit_react_and_pin() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let moder = server.account("moder", Role::Moderator).await;
    let general = room(&server, "general").await;
    let mut a = Sock::open(&server, &alice).await;
    let m = sent(&server, general, "hi", &alice).await;

    let (status, v) = edit(&server, m.id, "hi @bob", &alice).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["mentions"], json!([bob.user.user_id]));
    assert!(v["edited_at"].is_number());
    // Only the author edits, whatever the role.
    assert_eq!(
        edit(&server, m.id, "no", &bob).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        edit(&server, m.id, "no", &moder).await.0,
        StatusCode::FORBIDDEN
    );

    let react = format!("/api/messages/{}/reaction", m.id);
    let (status, _, v) = put(&server, &react, json!({ "emoji": "👍", "on": true }), &bob).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(
        v["reactions"],
        json!([{ "emoji": "👍", "user_ids": [bob.user.user_id] }])
    );
    let pin = format!("/api/messages/{}/pin", m.id);
    let (status, _, v) = put(&server, &pin, json!({ "on": true }), &bob).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["pinned"], true);
    let (status, _, v) = server
        .call(get(&format!("/api/rooms/{general}/pins"), &alice))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 1);
    let (_, _, v) = put(&server, &pin, json!({ "on": false }), &alice).await;
    assert_eq!(v["pinned"], false);

    // Each change pushed one MessageUpdated with the whole message, in order.
    let n = std::cell::Cell::new(0);
    let seen = a
        .until("four updates", |m| {
            n.set(n.get() + usize::from(matches!(m, ServerMsg::MessageUpdated { .. })));
            n.get() == 4
        })
        .await;
    let updates: Vec<&Message> = seen
        .iter()
        .filter_map(|m| match m {
            ServerMsg::MessageUpdated { message } => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(updates.len(), 4, "{seen:?}");
    assert!(updates[2].pinned && !updates[3].pinned);
    assert_eq!(updates[3].reactions.len(), 1);
    assert_eq!(updates[3].body, "hi @bob");
}

#[tokio::test]
async fn restricted_room_is_404_and_silent() {
    let server = TestServer::new().await;
    let owner = server.account("owner", Role::Owner).await;
    let bob = server.account("bob", Role::Member).await;
    let carol = server.account("carol", Role::Member).await;
    let moder = server.account("moder", Role::Moderator).await;
    let secret = restricted(&server, &owner, &[&bob]).await;
    let general = room(&server, "general").await;
    let mut c = Sock::open(&server, &carol).await;
    let b = Sock::open(&server, &bob).await;
    let mut m = Sock::open(&server, &moder).await;

    let hidden = sent(&server, secret, "psst", &bob).await;
    let routes = [
        (Method::GET, format!("/api/rooms/{secret}/messages"), None),
        (
            Method::GET,
            format!("/api/rooms/{secret}/messages?around={}", hidden.id),
            None,
        ),
        (Method::GET, format!("/api/rooms/{secret}/pins"), None),
        (
            Method::POST,
            format!("/api/rooms/{secret}/messages"),
            Some(json!({ "body": "x" })),
        ),
        (
            Method::PUT,
            format!("/api/rooms/{secret}/read"),
            Some(json!({ "last_read_id": hidden.id })),
        ),
        (
            Method::PATCH,
            format!("/api/messages/{}", hidden.id),
            Some(json!({ "body": "x" })),
        ),
        (Method::DELETE, format!("/api/messages/{}", hidden.id), None),
        (
            Method::PUT,
            format!("/api/messages/{}/reaction", hidden.id),
            Some(json!({ "emoji": "👍", "on": true })),
        ),
        (
            Method::PUT,
            format!("/api/messages/{}/pin", hidden.id),
            Some(json!({ "on": true })),
        ),
    ];
    for (method, path, body) in routes {
        let (status, _, v) = server
            .call(request(
                method.clone(),
                &path,
                body.as_ref(),
                Some(&carol.cookie),
            ))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}: {v}");
        assert_eq!(v["code"], "not_found");
    }
    // Same answer as a room that does not exist.
    assert_eq!(
        send(&server, 999_999, "x", &carol).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(history(&server, secret, &bob).await, vec![hidden.clone()]);
    // carol's socket gets neither the message nor typing; the moderator sees both.
    b.typing(secret);
    b.typing(general);
    sent(&server, general, "public", &bob).await;
    let seen = c.until_new("public").await;
    assert!(
        !seen.iter().any(|m| matches!(m,
            ServerMsg::MessageNew { message } if message.room_id == secret)
            || matches!(m, ServerMsg::Typing { room_id, .. } if *room_id == secret)),
        "{seen:?}"
    );
    m.until(
        "secret typing",
        |m| matches!(m, ServerMsg::Typing { room_id, .. } if *room_id == secret),
    )
    .await;
    // carol's own typing to the hidden room goes nowhere either.
    c.typing(secret);
    c.typing(general);
    let seen = m.until("general typing", |m| matches!(m, ServerMsg::Typing { room_id, user_id } if *room_id == general && *user_id == carol.user.user_id)).await;
    assert!(!seen.iter().any(|m| matches!(m, ServerMsg::Typing { room_id, user_id } if *room_id == secret && *user_id == carol.user.user_id)), "{seen:?}");
    // No unread entry for a hidden room.
    let ServerMsg::Hello { unread, .. } = Sock::open(&server, &carol).await.hello else {
        unreachable!()
    };
    assert!(!unread.iter().any(|u| u.room_id == secret));
}

#[tokio::test]
async fn mentions_count_in_unread() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let general = room(&server, "general").await;
    let before = Sock::open(&server, &bob).await.unread(general);
    assert_eq!((before.unread, before.mentions), (0, 0));
    let m = sent(&server, general, "hi @bob", &alice).await;
    assert_eq!(m.mentions, std::slice::from_ref(&bob.user.user_id));
    let b1 = Sock::open(&server, &bob).await;
    let u = b1.unread(general);
    assert_eq!((u.unread, u.mentions), (1, 1));
    let bob2 = server.session(&bob.user).await;
    let mut b2 = Sock::open(&server, &bob2).await;

    let read = format!("/api/rooms/{general}/read");
    let (status, _, _) = put(&server, &read, json!({ "last_read_id": m.id }), &bob).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    b2.until("read", |x| {
        *x == ServerMsg::Read {
            room_id: general,
            last_read_id: m.id,
        }
    })
    .await;
    let u = Sock::open(&server, &bob).await.unread(general);
    assert_eq!((u.unread, u.mentions, u.last_read_id), (0, 0, m.id));
    // Backwards is ignored, and Read says where the marker really is.
    let (status, _, _) = put(&server, &read, json!({ "last_read_id": 0 }), &bob).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    b2.until("read again", |x| {
        *x == ServerMsg::Read {
            room_id: general,
            last_read_id: m.id,
        }
    })
    .await;
    // The marker must name a message of this room: no parking it past every future message.
    let hangout = room(&server, "hangout").await;
    let other = sent(&server, hangout, "elsewhere", &alice).await;
    for bad in [i64::MAX, -1, other.id, other.id + 1000] {
        let (status, _, v) = put(&server, &read, json!({ "last_read_id": bad }), &bob).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }
    sent(&server, general, "later", &alice).await;
    assert_eq!(Sock::open(&server, &bob).await.unread(general).unread, 1);
}

#[tokio::test]
async fn mentions_skip_hidden_and_banned() {
    let server = TestServer::new().await;
    let owner = server.account("owner", Role::Owner).await;
    let bob = server.account("bob", Role::Member).await;
    let carol = server.account("carol", Role::Member).await;
    let dave = server.account("dave", Role::Member).await;
    let secret = restricted(&server, &owner, &[&bob, &dave]).await;
    let (status, _, _) = server
        .call(post(
            &format!("/api/users/{}/ban", dave.user.user_id),
            json!({}),
            &owner,
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    let m = sent(&server, secret, "@bob @carol @dave @nobody @owner", &bob).await;
    let mut want = vec![bob.user.user_id.clone(), owner.user.user_id.clone()];
    let mut got = m.mentions.clone();
    want.sort();
    got.sort();
    assert_eq!(got, want);
    // carol is let in later: the old mention never counts for her.
    let ids = [&bob.user.user_id, &carol.user.user_id];
    put(
        &server,
        &format!("/api/rooms/{secret}/members"),
        json!({ "user_ids": ids }),
        &owner,
    )
    .await;
    let u = Sock::open(&server, &carol).await.unread(secret);
    assert_eq!((u.unread, u.mentions), (1, 0));
    // A banned user keeps their messages but cannot post (their session is gone).
    let general = room(&server, "general").await;
    assert_eq!(
        send(&server, general, "x", &dave).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn typing_is_throttled_and_ephemeral() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let carol = server.account("carol", Role::Member).await;
    let general = room(&server, "general").await;
    let hangout = room(&server, "hangout").await;
    let mut a = Sock::open(&server, &alice).await;
    let mut c = Sock::open(&server, &carol).await;
    for _ in 0..3 {
        a.typing(general);
    }
    // Same socket, so processed after the three above.
    a.typing(hangout);
    let seen = c
        .until(
            "hangout typing",
            |m| matches!(m, ServerMsg::Typing { room_id, .. } if *room_id == hangout),
        )
        .await;
    let want = ServerMsg::Typing {
        room_id: general,
        user_id: alice.user.user_id.clone(),
    };
    assert_eq!(seen.iter().filter(|m| **m == want).count(), 1, "{seen:?}");
    // Not to the sender, no error back, and nothing stored.
    sent(&server, general, "done", &carol).await;
    let seen = a.until_new("done").await;
    assert!(
        !seen
            .iter()
            .any(|m| matches!(m, ServerMsg::Typing { .. } | ServerMsg::Error { .. })),
        "{seen:?}"
    );
    assert_eq!(history(&server, general, &carol).await.len(), 1);
}

#[tokio::test]
async fn reaction_validation() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let general = room(&server, "general").await;
    let m = sent(&server, general, "react", &alice).await;
    let path = format!("/api/messages/{}/reaction", m.id);
    let long = "👍".repeat(9);
    for bad in ["ab", " ", "", long.as_str()] {
        let (status, _, v) = put(&server, &path, json!({ "emoji": bad, "on": true }), &alice).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad:?}: {v}");
    }
    let (status, _, v) = put(&server, &path, json!({ "emoji": "👍", "on": true }), &alice).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, _, v) = put(
        &server,
        &path,
        json!({ "emoji": "👍", "on": false }),
        &alice,
    )
    .await;
    assert_eq!(v["reactions"], json!([]));
    // A body the server cannot parse gets the fixed text.
    let req = Request::builder()
        .method(Method::PUT)
        .uri(&path)
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, &alice.cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{\"emoji\": 1}"))
        .unwrap();
    let (status, _, v) = server.call(req).await;
    assert_eq!(
        (status, v["message"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid request body"))
    );
}

#[tokio::test]
async fn search_sees_only_visible_rooms() {
    let server = TestServer::new().await;
    let owner = server.account("owner", Role::Owner).await;
    let bob = server.account("bob", Role::Member).await;
    let carol = server.account("carol", Role::Member).await;
    let secret = restricted(&server, &owner, &[&bob]).await;
    let general = room(&server, "general").await;
    // Straight into the store: the send limit would stop 28 posts.
    let post = |room: i64, body: &'static str| {
        let s = server.state.store.clone();
        let bob = bob.user.user_id.clone();
        async move {
            s.post_message(vesper_store::NewMessage {
                room_id: room,
                author_id: &bob,
                body,
                reply_to: None,
                mentions: &[],
                attachments: &[],
                now: 1,
            })
            .await
            .unwrap()
            .id
        }
    };
    let mut open_ids = Vec::new();
    for _ in 0..3 {
        open_ids.push(post(general, "needle in general").await);
    }
    // Newer matches in the hidden room must neither show nor shorten carol's page.
    for _ in 0..25 {
        post(secret, "needle").await;
    }
    let search = |q: String, s: &Session| {
        let req = get(&format!("/api/search?{q}"), s);
        async { server.call(req).await }
    };
    let (status, _, v) = search("q=needle".into(), &carol).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let hits: Vec<i64> = v["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["message"]["id"].as_i64().unwrap())
        .collect();
    open_ids.reverse();
    assert_eq!(hits, open_ids);
    assert_eq!(v["next_before"], Value::Null);
    assert_eq!(v["hits"][0]["snippet"], "\u{2}needle\u{3} in general");
    // bob sees both rooms: a full page of 20 and a cursor.
    let (_, _, v) = search("q=needle".into(), &bob).await;
    assert_eq!(v["hits"].as_array().unwrap().len(), 20);
    assert!(v["next_before"].is_i64());

    // A hidden room id is the same 404 as an unknown one.
    let (status, _, hidden) = search(format!("q=needle&room_id={secret}"), &carol).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, _, unknown) = search("q=needle&room_id=999999".into(), &carol).await;
    assert_eq!(hidden, unknown);
    let (status, _, v) = search(format!("q=needle&room_id={secret}"), &bob).await;
    assert_eq!(
        (status, v["hits"].as_array().unwrap().len()),
        (StatusCode::OK, 20)
    );
}

#[tokio::test]
async fn search_hostile_input_and_rate_limit() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let general = room(&server, "general").await;
    sent(&server, general, "hello there", &alice).await;
    // `***`, `NEAR(`, `"`, `body:x`, `^` and an empty query: 200, never 500.
    for q in [
        "q=%2A%2A%2A",
        "q=NEAR%28",
        "q=%22",
        "q=body%3Ax",
        "q=%5E",
        "q=",
    ] {
        let (status, _, v) = server.call(get(&format!("/api/search?{q}"), &alice)).await;
        assert_eq!(status, StatusCode::OK, "{q}: {v}");
    }
    let (status, _, _) = server
        .call(get("/api/search?q=x&before=nope", &alice))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Burst of 10 per user, then 1 a second.
    let mut codes = Vec::new();
    for _ in 0..10 {
        codes.push(server.call(get("/api/search?q=hel", &alice)).await.0);
    }
    assert_eq!(codes[0], StatusCode::OK);
    assert_eq!(codes[9], StatusCode::TOO_MANY_REQUESTS, "{codes:?}");
}
