//! Signaling end to end: the real server on random ports, a real websocket client per tab (with
//! a session cookie and our Origin) and `vesper_sfu::testing::FakeClient` playing the browser's
//! WebRTC side.

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use common::{ORIGIN, PASSWORD, Session, TestServer, get, post, request, spawn_sfu};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use vesper_protocol::{
    CLOSE_SESSION_ENDED, ClientMsg, ErrorCode, MAX_MSG_BYTES, Peer, Role, RoomInfo, ServerMsg,
};
use vesper_sfu::testing::FakeClient;

/// Generous: CI machines can be slow and ICE/DTLS takes a few round trips.
const WAIT: Duration = Duration::from_secs(10);

type Ws = tokio_tungstenite::WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Open `/ws` with an optional cookie and Origin. `small_buffer` shrinks the client's TCP receive
/// buffer, so a client that stops reading backs the server up after a few messages.
async fn connect(
    addr: SocketAddr,
    cookie: Option<&str>,
    origin: Option<&str>,
    small_buffer: bool,
) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    let mut req = format!("ws://{addr}/ws").into_client_request().unwrap();
    if let Some(c) = cookie {
        req.headers_mut().insert("cookie", c.parse().unwrap());
    }
    if let Some(o) = origin {
        req.headers_mut().insert("origin", o.parse().unwrap());
    }
    let sock = tokio::net::TcpSocket::new_v4().unwrap();
    if small_buffer {
        sock.set_recv_buffer_size(4096).unwrap();
    }
    let tcp = sock.connect(addr).await.unwrap();
    let (ws, _) = tokio_tungstenite::client_async(req, MaybeTlsStream::Plain(tcp)).await?;
    Ok(ws)
}

fn http_status(r: Result<Ws, tokio_tungstenite::tungstenite::Error>) -> u16 {
    match r {
        Err(tokio_tungstenite::tungstenite::Error::Http(r)) => r.status().as_u16(),
        Err(e) => panic!("expected an HTTP refusal, got {e}"),
        Ok(_) => panic!("expected an HTTP refusal, the upgrade succeeded"),
    }
}

/// One websocket. A background task answers SFU renegotiation offers through the fake client
/// (so sessions are not closed for leaving an offer unanswered) and passes every other message on.
struct Conn {
    client: Arc<FakeClient>,
    out: mpsc::UnboundedSender<Message>,
    msgs: mpsc::UnboundedReceiver<ServerMsg>,
    task: JoinHandle<()>,
    freeze: Option<oneshot::Sender<()>>,
    thaw: Option<oneshot::Sender<()>>,
    /// The code of the server's close frame, once one arrived.
    close_code: Arc<Mutex<Option<u16>>>,
    hello: ServerMsg,
}

impl Conn {
    async fn open(server: &TestServer, s: &Session) -> Conn {
        Conn::open_with(server, s, false).await
    }

    async fn open_with(server: &TestServer, s: &Session, small_buffer: bool) -> Conn {
        let ws = connect(server.addr, Some(&s.cookie), Some(ORIGIN), small_buffer)
            .await
            .expect("websocket connects");
        let client = Arc::new(FakeClient::new().expect("fake client"));
        let (out, mut out_rx) = mpsc::unbounded_channel::<Message>();
        let (msgs_tx, mut msgs) = mpsc::unbounded_channel();
        let (freeze, mut frozen) = oneshot::channel::<()>();
        let (thaw, mut thawed) = oneshot::channel::<()>();
        let close_code = Arc::new(Mutex::new(None));
        let task = {
            let client = client.clone();
            let close_code = close_code.clone();
            tokio::spawn(async move {
                let (mut sink, mut stream) = ws.split();
                let mut was_frozen = false;
                loop {
                    tokio::select! {
                        // sink and stream stay alive (TCP open) but are not polled: pings go
                        // unanswered and nothing is read until thawed.
                        Ok(()) = &mut frozen, if !was_frozen => {
                            was_frozen = true;
                            let _ = (&mut thawed).await;
                        }
                        m = out_rx.recv() => match m {
                            Some(m) => if sink.send(m).await.is_err() { return },
                            None => return,
                        },
                        m = stream.next() => {
                            let text = match m {
                                Some(Ok(Message::Text(t))) => t,
                                Some(Ok(Message::Close(f))) => {
                                    *close_code.lock().unwrap() = f.map(|f| f.code.into());
                                    return;
                                }
                                Some(Err(_)) | None => return,
                                Some(Ok(_)) => continue,
                            };
                            let msg: ServerMsg = serde_json::from_str(&text).expect("valid ServerMsg");
                            match &msg {
                                ServerMsg::Offer { sdp } => {
                                    let sdp = client.accept_offer(sdp).expect("fake client answers");
                                    let a = serde_json::to_string(&ClientMsg::Answer { sdp }).unwrap();
                                    // A failed send (server closing) is fine: read on to the close frame.
                                    let _ = sink.send(Message::text(a)).await;
                                    continue;
                                }
                                ServerMsg::Joined { answer, .. } => {
                                    client.accept_answer(answer).expect("fake client accepts answer");
                                }
                                _ => {}
                            }
                            let _ = msgs_tx.send(msg);
                        }
                    }
                }
            })
        };
        let hello = tokio::time::timeout(WAIT, msgs.recv())
            .await
            .expect("Hello in time")
            .expect("socket open");
        assert!(
            matches!(hello, ServerMsg::Hello { .. }),
            "first message: {hello:?}"
        );
        Conn {
            client,
            out,
            msgs,
            task,
            freeze: Some(freeze),
            thaw: Some(thaw),
            close_code,
            hello,
        }
    }

    /// Stay connected but go silent, like a peer behind a dead NAT mapping.
    fn freeze(&mut self) {
        let _ = self.freeze.take().unwrap().send(());
    }

    fn thaw(&mut self) {
        let _ = self.thaw.take().unwrap().send(());
    }

    fn send(&self, m: &ClientMsg) {
        self.send_raw(serde_json::to_string(m).unwrap());
    }

    fn send_raw(&self, text: String) {
        self.out
            .send(Message::text(text))
            .expect("connection task alive");
    }

    fn join(&self, room_id: i64) {
        self.send(&ClientMsg::Join {
            room_id,
            offer: self.client.offer().into(),
        });
    }

    fn mute(&self, muted: bool) {
        self.send(&ClientMsg::SetVoiceState {
            muted,
            deafened: false,
        });
    }

    /// Next non-offer message; panics if none arrives in time or the socket closed.
    async fn next(&mut self) -> ServerMsg {
        tokio::time::timeout(WAIT, self.msgs.recv())
            .await
            .expect("message in time")
            .expect("socket still open")
    }

    /// Skip messages until one matches.
    async fn until(&mut self, what: &str, f: impl Fn(&ServerMsg) -> bool) -> ServerMsg {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let m = tokio::time::timeout_at(deadline, self.msgs.recv())
                .await
                .unwrap_or_else(|_| panic!("no {what} in time"))
                .unwrap_or_else(|| panic!("socket closed before {what}"));
            if f(&m) {
                return m;
            }
        }
    }

    /// Open a socket for `s`, join `room_id` and return (conn, user_id, peers).
    async fn joined(server: &TestServer, s: &Session, room_id: i64) -> (Conn, String, Vec<Peer>) {
        Conn::open(server, s).await.join_as(room_id).await
    }

    async fn join_as(mut self, room_id: i64) -> (Conn, String, Vec<Peer>) {
        self.join(room_id);
        match self.next().await {
            ServerMsg::Joined { user_id, peers, .. } => (self, user_id, peers),
            other => panic!("expected Joined, got {other:?}"),
        }
    }

    async fn expect_error(&mut self, code: ErrorCode) {
        match self.next().await {
            ServerMsg::Error { code: c, .. } => assert_eq!(c, code),
            other => panic!("expected Error {code:?}, got {other:?}"),
        }
    }

    /// Wait until the server closes the socket, skipping messages; returns the close code.
    async fn expect_closed_within(&mut self, wait: Duration) -> Option<u16> {
        let deadline = tokio::time::Instant::now() + wait;
        while tokio::time::timeout_at(deadline, self.msgs.recv())
            .await
            .expect("socket closes in time")
            .is_some()
        {}
        *self.close_code.lock().unwrap()
    }

    async fn expect_closed(&mut self) -> Option<u16> {
        self.expect_closed_within(WAIT).await
    }

    /// Messages up to (not including) the `PeerState` for `user_id`: a barrier that needs no
    /// sleep, since everything the server queued for this socket earlier arrives first.
    async fn until_state(&mut self, user_id: &str) -> Vec<ServerMsg> {
        let mut seen = vec![];
        loop {
            match self.next().await {
                ServerMsg::PeerState { user_id: u, .. } if u == user_id => return seen,
                m => seen.push(m),
            }
        }
    }

    /// Drop the TCP connection without a close frame or `Leave`.
    fn drop_abruptly(self) {
        self.task.abort();
    }
}

async fn wait_until(what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn rooms(server: &TestServer) -> Vec<RoomInfo> {
    server.state.store.rooms().await.unwrap()
}

async fn general(server: &TestServer) -> i64 {
    rooms(server)
        .await
        .into_iter()
        .find(|r| r.name == "general")
        .unwrap()
        .id
}

fn peer(s: &Session, room_id: i64) -> Peer {
    Peer {
        user_id: s.user.user_id.clone(),
        name: s.user.display_name.clone(),
        room_id,
        muted: false,
        deafened: false,
        force_muted: false,
    }
}

#[tokio::test]
async fn upgrade_without_cookie_is_401() {
    let server = TestServer::new().await;
    assert_eq!(
        http_status(connect(server.addr, None, Some(ORIGIN), false).await),
        401
    );
    let junk = format!("vesper_session={}", "0".repeat(64));
    assert_eq!(
        http_status(connect(server.addr, Some(&junk), Some(ORIGIN), false).await),
        401
    );
}

#[tokio::test]
async fn wrong_origin_upgrade_is_403() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    for origin in [Some("http://evil.example"), Some("null"), None] {
        let r = connect(server.addr, Some(&alice.cookie), origin, false).await;
        assert_eq!(http_status(r), 403, "{origin:?}");
    }
}

#[tokio::test]
async fn hello_is_first_message() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let c = Conn::open(&server, &alice).await;
    let ServerMsg::Hello {
        me,
        users,
        rooms,
        voice,
        unread,
    } = c.hello
    else {
        unreachable!()
    };
    assert_eq!(me.user.user_id, alice.user.user_id);
    assert_eq!(me.user.user_id.len(), 32);
    assert!(rooms.iter().any(|r| r.name == "general"), "{rooms:?}");
    assert!(users.iter().any(|u| u.user_id == alice.user.user_id));
    assert!(voice.is_empty() && unread.is_empty());
}

#[tokio::test]
async fn join_returns_answer_and_peer_list() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let (alice, bob) = (
        server.account("alice", Role::Member).await,
        server.account("bob", Role::Member).await,
    );
    let (mut a, a_id, peers) = Conn::joined(&server, &alice, room).await;
    assert!(peers.is_empty());
    assert_eq!(a_id, alice.user.user_id);

    let (b, b_id, peers) = Conn::joined(&server, &bob, room).await;
    assert_eq!(peers, vec![peer(&alice, room)]);
    assert_eq!(b_id, bob.user.user_id);
    match a.next().await {
        ServerMsg::PeerJoined { peer: p } => assert_eq!(p, peer(&bob, room)),
        other => panic!("expected PeerJoined, got {other:?}"),
    }
    // A socket opened later sees both in Hello.
    let carol = server.account("carol", Role::Member).await;
    let ServerMsg::Hello { voice, .. } = Conn::open(&server, &carol).await.hello else {
        unreachable!()
    };
    assert_eq!(voice.len(), 2, "{voice:?}");
    // The answers were real: media sessions come up and each side gets the other's track.
    wait_until("ICE connected", || {
        a.client.is_connected() && b.client.is_connected()
    })
    .await;
    wait_until("remote tracks", || {
        a.client.remote_tracks() == 1 && b.client.remote_tracks() == 1
    })
    .await;
}

#[tokio::test]
async fn voice_state_reaches_the_room() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let (mut a, _, _) = Conn::joined(&server, &alice, room).await;
    let (b, b_id, _) = Conn::joined(&server, &bob, room).await;
    b.send(&ClientMsg::SetVoiceState {
        muted: true,
        deafened: true,
    });
    let m = a
        .until("PeerState", |m| matches!(m, ServerMsg::PeerState { .. }))
        .await;
    assert_eq!(
        m,
        ServerMsg::PeerState {
            user_id: b_id,
            muted: true,
            deafened: true,
            force_muted: false
        }
    );
}

/// A second tab of the same account joining voice replaces the first tab's voice session.
#[tokio::test]
async fn second_session_for_same_user_replaces_first() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let (mut a1, a_id, _) = Conn::joined(&server, &alice, room).await;
    let (mut b, _, _) = Conn::joined(&server, &bob, room).await;
    assert!(matches!(a1.next().await, ServerMsg::PeerJoined { .. }));

    let (a2, a2_id, peers) = Conn::joined(&server, &alice, room).await;
    assert_eq!(a2_id, a_id);
    assert_eq!(peers.len(), 1, "only bob: {peers:?}");
    a1.expect_closed().await;

    a2.mute(true);
    let seen = b.until_state(&a_id).await;
    assert!(
        matches!(&seen[..], [ServerMsg::PeerLeft { user_id: l }, ServerMsg::PeerJoined { peer }]
            if *l == a_id && peer.user_id == a_id),
        "{seen:?}"
    );

    let carol = server.account("carol", Role::Member).await;
    let (_c, _, peers) = Conn::joined(&server, &carol, room).await;
    assert_eq!(peers.iter().filter(|p| p.user_id == a_id).count(), 1);
    assert_eq!(peers.len(), 2);
}

#[tokio::test]
async fn room_full_rejects_thirteenth() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let mut members = vec![];
    for i in 0..12 {
        let s = server.account(&format!("user{i}"), Role::Member).await;
        let (c, id, peers) = Conn::joined(&server, &s, room).await;
        assert_eq!(peers.len(), i);
        members.push((c, id));
    }
    let late = server.account("user12", Role::Member).await;
    let mut late = Conn::open(&server, &late).await;
    late.join(room);
    late.expect_error(ErrorCode::RoomFull).await;

    let (first, first_id) = &members[0];
    first.mute(true);
    let first_id = first_id.clone();
    for (i, (c, _)) in members.iter_mut().enumerate().skip(1) {
        let seen = c.until_state(&first_id).await;
        // Only the PeerJoined of later members; nothing about user12, nobody left.
        assert_eq!(seen.len(), 11 - i, "{seen:?}");
        assert!(
            seen.iter()
                .all(|m| matches!(m, ServerMsg::PeerJoined { peer } if peer.name != "user12")),
            "{seen:?}"
        );
    }
}

#[tokio::test]
async fn malformed_and_oversized_messages_do_not_crash() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let alice = server.account("alice", Role::Member).await;
    let xavier = server.account("xavier", Role::Member).await;
    let (mut a, _, _) = Conn::joined(&server, &alice, room).await;

    let mut x = Conn::open(&server, &xavier).await;
    x.send_raw("this is not json".into());
    x.expect_error(ErrorCode::BadMessage).await;
    x.send_raw(r#"{"type":"nope"}"#.into());
    x.expect_error(ErrorCode::BadMessage).await;
    x.send_raw(r#"{"type":"join","room":"main","name":"x","key":"k","offer":"o"}"#.into());
    x.expect_error(ErrorCode::BadMessage).await;
    x.join(999_999);
    x.expect_error(ErrorCode::BadMessage).await;
    x.mute(true);
    x.expect_error(ErrorCode::BadMessage).await;
    x.send(&ClientMsg::Join {
        room_id: room,
        offer: "v=0 not an sdp".into(),
    });
    x.expect_error(ErrorCode::BadMessage).await;
    // Many failures do not close an authenticated socket.
    for _ in 0..10 {
        x.send_raw("{".into());
        x.expect_error(ErrorCode::BadMessage).await;
    }

    // Still usable after all that.
    let (mut x, x_id, _) = x.join_as(room).await;
    assert!(matches!(a.next().await, ServerMsg::PeerJoined { .. }));

    x.send_raw("x".repeat(MAX_MSG_BYTES + 1));
    x.expect_closed().await;
    match a.next().await {
        ServerMsg::PeerLeft { user_id } => assert_eq!(user_id, x_id),
        other => panic!("expected PeerLeft, got {other:?}"),
    }
    let yuki = server.account("yuki", Role::Member).await;
    let (_y, _, peers) = Conn::joined(&server, &yuki, room).await;
    assert_eq!(peers.len(), 1);
}

#[tokio::test]
async fn abrupt_close_removes_presence() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let (mut a, _, _) = Conn::joined(&server, &alice, room).await;
    let (b, b_id, _) = Conn::joined(&server, &bob, room).await;
    assert!(matches!(a.next().await, ServerMsg::PeerJoined { .. }));

    b.drop_abruptly();
    let got = tokio::time::timeout(Duration::from_secs(15), a.msgs.recv())
        .await
        .expect("PeerLeft within 15 s");
    match got {
        Some(ServerMsg::PeerLeft { user_id }) => assert_eq!(user_id, b_id),
        other => panic!("expected PeerLeft, got {other:?}"),
    }
}

#[tokio::test]
async fn revoked_session_closes_socket() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let phone = server.session(&alice.user).await;
    let mut on_phone = Conn::open(&server, &phone).await;
    let mut on_laptop = Conn::open(&server, &alice).await;
    let (status, _, devices) = server.call(get("/api/sessions", &alice)).await;
    assert_eq!(status, StatusCode::OK);
    let other = devices
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["current"] == false)
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let path = format!("/api/sessions/{other}");
    let (status, _, _) = server
        .call(request(Method::DELETE, &path, None, Some(&alice.cookie)))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        on_phone.expect_closed_within(Duration::from_secs(1)).await,
        Some(CLOSE_SESSION_ENDED)
    );
    // The laptop's socket is untouched: it still gets events.
    let (status, _, _) = server
        .call(request(
            Method::PATCH,
            "/api/me",
            Some(&json!({ "display_name": "Alice" })),
            Some(&alice.cookie),
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    on_laptop
        .until(
            "UserUpdated",
            |m| matches!(m, ServerMsg::UserUpdated { user } if user.display_name == "Alice"),
        )
        .await;
    // Logout ends the last one.
    let (status, _, _) = server.call(post("/api/logout", json!({}), &alice)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(on_laptop.expect_closed().await, Some(CLOSE_SESSION_ENDED));
}

#[tokio::test]
async fn password_change_closes_other_sessions_only() {
    let server = TestServer::new().await;
    let first = server.owner().await;
    let second = server.login("owner").await;
    let mut a = Conn::open(&server, &first).await;
    let mut b = Conn::open(&server, &second).await;
    let change = json!({ "old": PASSWORD, "new": "test-password-2" });
    let (status, _, v) = server.call(post("/api/me/password", change, &first)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(
        b.expect_closed_within(Duration::from_secs(1)).await,
        Some(CLOSE_SESSION_ENDED)
    );
    let (status, _, _) = server
        .call(request(
            Method::PATCH,
            "/api/me",
            Some(&json!({ "display_name": "Owner" })),
            Some(&first.cookie),
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    a.until("UserUpdated", |m| {
        matches!(m, ServerMsg::UserUpdated { .. })
    })
    .await;
}

#[tokio::test]
async fn ban_closes_socket_and_voice() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let owner = server.account("owner", Role::Owner).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let (mut a, a_id, _) = Conn::joined(&server, &alice, room).await;
    let (mut b, _, _) = Conn::joined(&server, &bob, room).await;
    let path = format!("/api/users/{a_id}/ban");
    let (status, _, v) = server.call(post(&path, json!({}), &owner)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(
        a.expect_closed_within(Duration::from_secs(1)).await,
        Some(CLOSE_SESSION_ENDED)
    );
    // Voice is left, and the ban reaches everyone as a user update (in either order).
    let (mut left, mut updated) = (false, false);
    while !(left && updated) {
        match b.next().await {
            ServerMsg::PeerLeft { user_id } if user_id == a_id => left = true,
            ServerMsg::UserUpdated { user } if user.user_id == a_id => {
                assert!(user.banned);
                updated = true;
            }
            _ => {}
        }
    }
    // The cookie cannot reconnect.
    assert_eq!(
        http_status(connect(server.addr, Some(&alice.cookie), Some(ORIGIN), false).await),
        401
    );
}

#[tokio::test]
async fn restricted_room_changes_reach_only_members() {
    let server = TestServer::new().await;
    let owner = server.account("owner", Role::Owner).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let mut a = Conn::open(&server, &alice).await;
    let mut b = Conn::open(&server, &bob).await;
    let (status, _, room) = server
        .call(post("/api/rooms", json!({ "name": "secret" }), &owner))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = room["id"].as_i64().unwrap();
    let path = format!("/api/rooms/{id}");
    let (status, _, _) = server
        .call(request(
            Method::PATCH,
            &path,
            Some(&json!({ "restricted": true })),
            Some(&owner.cookie),
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    let path = format!("/api/rooms/{id}/members");
    let body = json!({ "user_ids": [bob.user.user_id] });
    let (status, _, _) = server
        .call(request(
            Method::PUT,
            &path,
            Some(&body),
            Some(&owner.cookie),
        ))
        .await;
    assert_eq!(status, StatusCode::OK);

    let has_secret = |m: &ServerMsg| match m {
        ServerMsg::RoomsChanged { rooms } => rooms.iter().any(|r| r.name == "secret"),
        other => panic!("expected RoomsChanged, got {other:?}"),
    };
    // Created (open), restricted, members set.
    let alice_sees: Vec<bool> = vec![
        has_secret(&a.next().await),
        has_secret(&a.next().await),
        has_secret(&a.next().await),
    ];
    assert_eq!(alice_sees, [true, false, false]);
    let bob_sees: Vec<bool> = vec![
        has_secret(&b.next().await),
        has_secret(&b.next().await),
        has_secret(&b.next().await),
    ];
    assert_eq!(bob_sees, [true, false, true]);
    // Voice in a room you cannot see is refused.
    a.join(id);
    a.expect_error(ErrorCode::Forbidden).await;
}

#[tokio::test]
async fn full_queue_closes_only_that_socket() {
    let server = TestServer::new().await;
    let owner = server.account("owner", Role::Owner).await;
    // Big RoomsChanged messages (about 3 KB each), so the stalled socket backs up quickly.
    for i in 0..30 {
        let name = format!("{i:02}{}", "x".repeat(30));
        let (status, _, _) = server
            .call(post("/api/rooms", json!({ "name": name }), &owner))
            .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let general = general(&server).await;
    let mut fast = Conn::open(&server, &owner).await;
    let mut slow = Conn::open_with(&server, &owner, true).await;
    slow.freeze();
    const N: usize = 1000;
    let path = format!("/api/rooms/{general}");
    for i in 0..N {
        let body = json!({ "name": format!("general{}", i % 2) });
        let (status, _, _) = server
            .call(request(
                Method::PATCH,
                &path,
                Some(&body),
                Some(&owner.cookie),
            ))
            .await;
        assert_eq!(status, StatusCode::OK);
    }
    for _ in 0..N {
        assert!(matches!(fast.next().await, ServerMsg::RoomsChanged { .. }));
    }
    slow.thaw();
    let mut got = 0;
    while tokio::time::timeout(WAIT, slow.msgs.recv())
        .await
        .expect("slow socket ends in time")
        .is_some()
    {
        got += 1;
    }
    assert!(got < N, "the stalled socket was not closed: got all {got}");
    // The other socket still works.
    let (status, _, _) = server
        .call(request(
            Method::PATCH,
            &path,
            Some(&json!({ "name": "general" })),
            Some(&owner.cookie),
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(matches!(fast.next().await, ServerMsg::RoomsChanged { .. }));
}

#[tokio::test]
async fn resolve_public_returns_loopback() {
    let addrs = vesper_server::public_addr::resolve_public("localhost", 40000).await;
    assert!(
        addrs
            .iter()
            .any(|a| a.ip().is_loopback() && a.port() == 40000),
        "{addrs:?}"
    );
}

async fn health(addr: SocketAddr) -> String {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut resp = String::new();
    s.read_to_string(&mut resp).await.unwrap();
    resp
}

#[tokio::test]
async fn health_returns_ok() {
    let resp = health(TestServer::new().await.addr).await;
    assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
    assert!(resp.ends_with("\r\n\r\nok"), "{resp}");
}

#[tokio::test]
async fn dead_sfu_fails_health_and_joins_say_why() {
    let sfu = spawn_sfu();
    let server = TestServer::with(|_| {}, sfu.clone()).await;
    let alice = server.account("alice", Role::Member).await;
    assert!(health(server.addr).await.starts_with("HTTP/1.1 200"));
    sfu.crash_for_test();
    wait_until("SFU death", || !sfu.is_alive()).await;
    let resp = health(server.addr).await;
    assert!(resp.starts_with("HTTP/1.1 503"), "{resp}");
    assert!(resp.ends_with("\r\n\r\nsfu down"), "{resp}");
    let mut c = Conn::open(&server, &alice).await;
    c.join(general(&server).await);
    match c.next().await {
        ServerMsg::Error { code, message } => {
            assert_eq!(code, ErrorCode::BadMessage);
            assert_eq!(message, "server voice engine unavailable");
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn unanswered_pings_remove_presence() {
    let server = TestServer::with(
        |c| c.ping_interval = Duration::from_millis(200),
        spawn_sfu(),
    )
    .await;
    let room = general(&server).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let (mut a, _, _) = Conn::joined(&server, &alice, room).await;
    let (mut b, b_id, _) = Conn::joined(&server, &bob, room).await;
    assert!(matches!(a.next().await, ServerMsg::PeerJoined { .. }));
    // Renegotiation done, so the SFU's answer timeout cannot be what removes bob.
    wait_until("tracks negotiated", || {
        a.client.remote_tracks() == 1 && b.client.remote_tracks() == 1
    })
    .await;
    b.freeze();
    // `a` keeps answering pings, so it outlives several intervals and sees bob go, well before
    // the SFU's ICE (~19 s) or answer (10 s) timeouts could.
    let left = tokio::time::timeout(Duration::from_secs(3), a.msgs.recv()).await;
    match left.expect("PeerLeft within 3 s").expect("a still open") {
        ServerMsg::PeerLeft { user_id } => assert_eq!(user_id, b_id),
        other => panic!("expected PeerLeft, got {other:?}"),
    }
}

#[tokio::test]
async fn socket_cap_rejects_extra_upgrade_and_releases_slot() {
    let server = TestServer::with(|c| c.max_sockets = 2, spawn_sfu()).await;
    let alice = server.account("alice", Role::Member).await;
    let _a = Conn::open(&server, &alice).await;
    let b = Conn::open(&server, &alice).await;
    let third = connect(server.addr, Some(&alice.cookie), Some(ORIGIN), false).await;
    assert_eq!(http_status(third), 503);
    b.drop_abruptly();
    let deadline = Instant::now() + WAIT;
    while connect(server.addr, Some(&alice.cookie), Some(ORIGIN), false)
        .await
        .is_err()
    {
        assert!(Instant::now() < deadline, "slot never released");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Page refresh: the old socket drops while the new one joins as the same user. Whatever
/// order the server sees them in, the old session's SFU leave must not end the new session.
// Multi-threaded so the two sockets' handlers really run in parallel on the server.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejoin_while_old_socket_closes_keeps_new_session() {
    for i in 0..10 {
        let server = TestServer::new().await;
        let room = general(&server).await;
        let alice = server.account("alice", Role::Member).await;
        let bob = server.account("bob", Role::Member).await;
        let (mut b, b_id, _) = Conn::joined(&server, &bob, room).await;
        let (a1, _, _) = Conn::joined(&server, &alice, room).await;
        assert!(matches!(b.next().await, ServerMsg::PeerJoined { .. }));
        let a2 = Conn::open(&server, &alice).await;
        a1.drop_abruptly();
        if i % 2 == 0 {
            // Join right after the server has dropped the old session from presence, which
            // is when its SFU leave used to be still in flight.
            match b.next().await {
                ServerMsg::PeerLeft { .. } => {}
                other => panic!("iteration {i}: expected PeerLeft, got {other:?}"),
            }
        }
        let (mut a2, _, peers) = a2.join_as(room).await;
        assert_eq!(peers.len(), 1, "iteration {i}: {peers:?}");
        wait_until("new session media", || {
            a2.client.is_connected() && b.client.is_connected() && a2.client.remote_tracks() >= 1
        })
        .await;
        b.client.send_audio(10);
        wait_until("audio reaches the new session", || {
            !a2.client.received().is_empty()
        })
        .await;
        b.mute(true);
        let seen = a2.until_state(&b_id).await;
        assert!(seen.is_empty(), "iteration {i}: {seen:?}");
        while let Ok(m) = b.msgs.try_recv() {
            assert!(
                !matches!(m, ServerMsg::Error { .. }),
                "iteration {i}: {m:?}"
            );
        }
    }
}

#[tokio::test]
async fn watch_sfu_fires_after_crash_and_not_while_healthy() {
    let sfu = spawn_sfu();
    let (tx, mut rx) = mpsc::unbounded_channel();
    vesper_server::watch_sfu(sfu.clone(), Duration::from_millis(20), move || {
        let _ = tx.send(());
    });
    // Healthy for many intervals: nothing fires.
    assert!(
        tokio::time::timeout(Duration::from_millis(300), rx.recv())
            .await
            .is_err(),
        "on_dead fired for a healthy SFU"
    );
    sfu.crash_for_test();
    tokio::time::timeout(WAIT, rx.recv())
        .await
        .expect("on_dead did not fire after crash")
        .unwrap();
    // FnOnce: exactly once, then the sender is dropped.
    assert!(rx.recv().await.is_none());
}
