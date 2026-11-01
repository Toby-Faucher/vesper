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
use serde_json::Value;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use vesper_protocol::{
    CLOSE_DISCONNECTED, CLOSE_SESSION_ENDED, ClientMsg, ErrorCode, MAX_MSG_BYTES, Peer, Role,
    RoomInfo, ServerMsg,
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

    /// Join; presence events that were already on the way (this socket hears every room it
    /// can see) are skipped.
    async fn join_as(mut self, room_id: i64) -> (Conn, String, Vec<Peer>) {
        self.join(room_id);
        loop {
            match self.next().await {
                ServerMsg::Joined { user_id, peers, .. } => return (self, user_id, peers),
                ServerMsg::PeerJoined { .. }
                | ServerMsg::PeerLeft { .. }
                | ServerMsg::PeerState { .. } => {}
                other => panic!("expected Joined, got {other:?}"),
            }
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
    assert!(voice.is_empty());
    // One entry per visible room, nothing unread on a fresh server.
    let ids: Vec<i64> = rooms.iter().map(|r| r.id).collect();
    let unread_ids: Vec<i64> = unread.iter().map(|u| u.room_id).collect();
    assert_eq!(unread_ids, ids);
    assert!(unread.iter().all(|u| u.unread == 0 && u.mentions == 0));
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

/// A second tab of the same account joining voice takes the call over. The first tab's socket
/// stays open for chat and gets `PeerLeft` for itself.
#[tokio::test]
async fn second_tab_join_replaces_first_voice_session() {
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
    assert_eq!(
        a1.next().await,
        ServerMsg::PeerLeft {
            user_id: a_id.clone()
        }
    );
    // Still open: it sees the new session's presence like any other socket.
    match a1.next().await {
        ServerMsg::PeerJoined { peer } => assert_eq!(peer.user_id, a_id),
        other => panic!("expected PeerJoined, got {other:?}"),
    }

    a2.mute(true);
    let seen = b.until_state(&a_id).await;
    assert!(
        matches!(&seen[..], [ServerMsg::PeerLeft { user_id: l }, ServerMsg::PeerJoined { peer }]
            if *l == a_id && peer.user_id == a_id),
        "{seen:?}"
    );
    let carol = server.account("carol", Role::Member).await;
    let ServerMsg::Hello { voice, .. } = Conn::open(&server, &carol).await.hello else {
        unreachable!()
    };
    assert_eq!(voice.iter().filter(|p| p.user_id == a_id).count(), 1);
    assert_eq!(voice.len(), 2, "{voice:?}");

    // The first tab may join again: its old session is gone, so this is not an "already
    // joined" refusal (the offer is junk so the fake client is not reused).
    a1.send(&ClientMsg::Join {
        room_id: room,
        offer: "v=0 not an sdp".into(),
    });
    match a1
        .until("Error", |m| matches!(m, ServerMsg::Error { .. }))
        .await
    {
        ServerMsg::Error { message, .. } => assert_eq!(message, "invalid offer"),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn presence_reaches_sockets_outside_the_room() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let alice = server.account("alice", Role::Member).await;
    let carol = server.account("carol", Role::Member).await;
    let mut c = Conn::open(&server, &carol).await;
    let (a, a_id, _) = Conn::joined(&server, &alice, room).await;
    assert_eq!(
        c.next().await,
        ServerMsg::PeerJoined {
            peer: peer(&alice, room)
        }
    );
    a.send(&ClientMsg::Leave);
    assert_eq!(c.next().await, ServerMsg::PeerLeft { user_id: a_id });
}

#[tokio::test]
async fn voice_state_broadcasts_deafened() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let alice = server.account("alice", Role::Member).await;
    let carol = server.account("carol", Role::Member).await;
    let mut c = Conn::open(&server, &carol).await;
    let (a, a_id, _) = Conn::joined(&server, &alice, room).await;
    a.send(&ClientMsg::SetVoiceState {
        muted: false,
        deafened: true,
    });
    let m = c
        .until("PeerState", |m| matches!(m, ServerMsg::PeerState { .. }))
        .await;
    assert_eq!(
        m,
        ServerMsg::PeerState {
            user_id: a_id,
            muted: false,
            deafened: true,
            force_muted: false
        }
    );
}

/// The owner makes a restricted room listing `members`; returns its id.
async fn restricted_room(server: &TestServer, owner: &Session, members: &[&Session]) -> i64 {
    let id = open_room(server, owner).await;
    restrict(server, owner, id, members).await;
    id
}

/// The owner makes an open room named `secret`; returns its id.
async fn open_room(server: &TestServer, owner: &Session) -> i64 {
    let (status, _, room) = server
        .call(post("/api/rooms", json!({ "name": "secret" }), owner))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    room["id"].as_i64().unwrap()
}

/// Make room `id` restricted to `members` (the list first, so listed people never lose sight).
async fn restrict(server: &TestServer, owner: &Session, id: i64, members: &[&Session]) {
    set_members(server, owner, id, members).await;
    let path = format!("/api/rooms/{id}");
    let body = json!({ "restricted": true });
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

async fn set_members(server: &TestServer, owner: &Session, room: i64, members: &[&Session]) {
    let ids: Vec<&str> = members.iter().map(|s| s.user.user_id.as_str()).collect();
    let path = format!("/api/rooms/{room}/members");
    let body = json!({ "user_ids": ids });
    let (status, _, v) = server
        .call(request(
            Method::PUT,
            &path,
            Some(&body),
            Some(&owner.cookie),
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
}

fn is_rooms_changed(m: &ServerMsg) -> bool {
    matches!(m, ServerMsg::RoomsChanged { .. })
}

#[tokio::test]
async fn restricted_room_presence_is_private() {
    let server = TestServer::new().await;
    let open = general(&server).await;
    let owner = server.account("owner", Role::Owner).await;
    let bob = server.account("bob", Role::Member).await;
    let carol = server.account("carol", Role::Member).await;
    let alice = server.account("alice", Role::Member).await;
    let secret = restricted_room(&server, &owner, &[&bob]).await;
    let mut c = Conn::open(&server, &carol).await;
    let mut o = Conn::open(&server, &owner).await;
    let (_b, b_id, _) = Conn::joined(&server, &bob, secret).await;
    // Staff see it.
    match o.next().await {
        ServerMsg::PeerJoined { peer } => assert_eq!(peer.user_id, b_id),
        other => panic!("expected PeerJoined, got {other:?}"),
    }
    // Barrier: alice's join in the open room is the first thing carol hears.
    let (_a, a_id, _) = Conn::joined(&server, &alice, open).await;
    match c.next().await {
        ServerMsg::PeerJoined { peer } => assert_eq!(peer.user_id, a_id),
        other => panic!("expected alice's PeerJoined, got {other:?}"),
    }
    let ServerMsg::Hello { voice, .. } = Conn::open(&server, &carol).await.hello else {
        unreachable!()
    };
    assert_eq!(voice.len(), 1, "only alice: {voice:?}");
    c.join(secret);
    let hidden = c.next().await;
    // An unknown room id is refused the same way, word for word.
    c.join(999_999);
    let unknown = c.next().await;
    assert!(
        matches!(
            &hidden,
            ServerMsg::Error {
                code: ErrorCode::Forbidden,
                ..
            }
        ),
        "{hidden:?}"
    );
    assert_eq!(hidden, unknown);
}

/// Presence messages (peer_joined, peer_left, peer_state) up to, not including, `user_id`'s
/// `PeerJoined`: a barrier, since everything queued earlier for this socket arrives first.
async fn peer_msgs_until_joined(c: &mut Conn, user_id: &str) -> Vec<ServerMsg> {
    let mut seen = vec![];
    loop {
        match c.next().await {
            ServerMsg::PeerJoined { peer } if peer.user_id == user_id => return seen,
            m @ (ServerMsg::PeerJoined { .. }
            | ServerMsg::PeerLeft { .. }
            | ServerMsg::PeerState { .. }) => seen.push(m),
            _ => {}
        }
    }
}

/// An open room becomes restricted while people are in it and watching it.
#[tokio::test]
async fn presence_stops_when_room_becomes_restricted() {
    let server = TestServer::new().await;
    let open = general(&server).await;
    let owner = server.account("owner", Role::Owner).await;
    let [alice, bob, carol, dave, eve, frank] = {
        let mut v = vec![];
        for n in ["alice", "bob", "carol", "dave", "eve", "frank"] {
            v.push(server.account(n, Role::Member).await);
        }
        v.try_into().ok().unwrap()
    };
    let secret = open_room(&server, &owner).await;
    let mut c = Conn::open(&server, &carol).await;
    let mut d = Conn::open(&server, &dave).await;
    let (_a, _, _) = Conn::joined(&server, &alice, secret).await;
    let (mut e, e_id, _) = Conn::joined(&server, &eve, secret).await;
    // Carol sees the open room's voice.
    assert_eq!(peer_msgs_until_joined(&mut c, &e_id).await.len(), 1);

    // Eve is not listed: she is taken out of voice.
    restrict(&server, &owner, secret, &[&alice, &bob, &dave]).await;
    e.until(
        "own PeerLeft",
        |m| matches!(m, ServerMsg::PeerLeft { user_id } if *user_id == e_id),
    )
    .await;
    assert!(
        !server
            .state
            .voice
            .peers(|_| true)
            .iter()
            .any(|p| p.user_id == e_id)
    );
    let (b, b_id, _) = Conn::joined(&server, &bob, secret).await;
    // Dave is listed and sees bob arrive.
    peer_msgs_until_joined(&mut d, &b_id).await;

    // Dave is taken off the list: bob's mute no longer reaches him.
    set_members(&server, &owner, secret, &[&alice, &bob]).await;
    b.mute(true);
    let (_f, f_id, _) = Conn::joined(&server, &frank, open).await;
    let seen = peer_msgs_until_joined(&mut c, &f_id).await;
    assert!(seen.is_empty(), "carol: {seen:?}");
    let seen = peer_msgs_until_joined(&mut d, &f_id).await;
    assert!(seen.is_empty(), "dave: {seen:?}");
}

/// A join that read the room before it was restricted must not put the old copy back: presence
/// follows the new member list, and a joiner no longer listed is refused.
#[tokio::test]
async fn join_with_room_read_before_restrict_follows_new_rule() {
    let server = TestServer::new().await;
    let open = general(&server).await;
    let owner = server.account("owner", Role::Owner).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let carol = server.account("carol", Role::Member).await;
    let dave = server.account("dave", Role::Member).await;
    let secret = open_room(&server, &owner).await;
    let stale = rooms(&server)
        .await
        .into_iter()
        .find(|r| r.id == secret)
        .unwrap();
    let mut c = Conn::open(&server, &carol).await;
    // The change lands between the join's read and its insert.
    restrict(&server, &owner, secret, &[&bob]).await;

    let voice = &server.state.voice;
    let (tx, _rx) = vesper_server::hub::outbox();
    let client = FakeClient::new().unwrap();
    let r = voice
        .join(&bob.user, &stale, client.offer().into(), tx)
        .await;
    assert!(r.is_ok(), "{r:?}");
    let (_a, a_id, _) = Conn::joined(&server, &alice, open).await;
    let seen = peer_msgs_until_joined(&mut c, &a_id).await;
    assert!(seen.is_empty(), "carol: {seen:?}");

    let (tx, _rx2) = vesper_server::hub::outbox();
    let client2 = FakeClient::new().unwrap();
    let r = voice
        .join(&dave.user, &stale, client2.offer().into(), tx)
        .await;
    assert_eq!(r, Err(ErrorCode::Forbidden));
    assert_eq!(voice.peers(|_| true).len(), 2, "bob and alice");
}

/// A second tab's join that fails before the SFU leaves the first tab's call up.
#[tokio::test]
async fn failed_second_join_keeps_first_call() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let (mut a1, a_id, _) = Conn::joined(&server, &alice, room).await;
    let (b, b_id, _) = Conn::joined(&server, &bob, room).await;
    let mut a2 = Conn::open(&server, &alice).await;
    a2.send(&ClientMsg::Join {
        room_id: room,
        offer: "v=0 not an sdp".into(),
    });
    a2.until("Error", |m| matches!(m, ServerMsg::Error { .. }))
        .await;
    b.mute(true);
    let seen = a1.until_state(&b_id).await;
    assert!(
        !seen.iter().any(|m| matches!(m, ServerMsg::PeerLeft { .. })),
        "{seen:?}"
    );
    assert!(
        server
            .state
            .voice
            .peers(|_| true)
            .iter()
            .any(|p| p.user_id == a_id)
    );
}

#[tokio::test]
async fn twelfth_joins_thirteenth_is_full() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let hangout = rooms(&server)
        .await
        .into_iter()
        .find(|r| r.name == "hangout")
        .unwrap()
        .id;
    let mut members = vec![];
    let mut sessions = vec![];
    for i in 0..12 {
        let s = server.account(&format!("user{i}"), Role::Member).await;
        let (c, id, peers) = Conn::joined(&server, &s, room).await;
        assert_eq!(peers.len(), i);
        members.push((c, id));
        sessions.push(s);
    }
    let late_s = server.account("user12", Role::Member).await;
    let mut late = Conn::open(&server, &late_s).await;
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

    // user12 in another room: a second tab refused by the full room leaves that call alone.
    let (mut late, late_id, _) = late.join_as(hangout).await;
    let mut late2 = Conn::open(&server, &late_s).await;
    late2.join(room);
    late2.expect_error(ErrorCode::RoomFull).await;
    let (second, second_id) = &members[1];
    second.mute(true);
    let seen = late.until_state(second_id).await;
    assert!(
        !seen.iter().any(|m| matches!(m, ServerMsg::PeerLeft { .. })),
        "{seen:?}"
    );
    let peers = server.state.voice.peers(|_| true);
    assert!(
        peers
            .iter()
            .any(|p| p.user_id == late_id && p.room_id == hangout)
    );

    // A member's own second tab still fits: it replaces their place.
    let (_t, id, peers) = Conn::joined(&server, &sessions[0], room).await;
    assert_eq!(id, members[0].1);
    assert_eq!(peers.len(), 11);
}

/// Two joins race for the last place: exactly one gets it. Both run on one task, so without the
/// join lock both would pass the size check while the SFU answers the first.
#[tokio::test]
async fn two_joins_race_for_the_twelfth_place() {
    let server = TestServer::new().await;
    let room = rooms(&server)
        .await
        .into_iter()
        .find(|r| r.name == "general")
        .unwrap();
    let voice = &server.state.voice;
    let mut keep = vec![];
    let mut join = async |name: String| {
        let s = server.account(&name, Role::Member).await;
        let (tx, rx) = vesper_server::hub::outbox();
        let client = FakeClient::new().unwrap();
        let offer = client.offer().to_string();
        keep.push((rx, client));
        (s, tx, offer)
    };
    for i in 0..11 {
        let (s, tx, offer) = join(format!("user{i}")).await;
        voice.join(&s.user, &room, offer, tx).await.unwrap();
    }
    let (x, x_tx, x_offer) = join("userx".into()).await;
    let (y, y_tx, y_offer) = join("usery".into()).await;
    let (rx, ry) = tokio::join!(
        voice.join(&x.user, &room, x_offer, x_tx),
        voice.join(&y.user, &room, y_offer, y_tx),
    );
    assert!(
        matches!(
            (&rx, &ry),
            (Ok(_), Err(ErrorCode::RoomFull)) | (Err(ErrorCode::RoomFull), Ok(_))
        ),
        "{rx:?} {ry:?}"
    );
    assert_eq!(voice.peers(|_| true).len(), 12);
}

/// Losing sight of a room (member list or demotion) takes you out of its voice.
#[tokio::test]
async fn losing_access_leaves_voice() {
    let server = TestServer::new().await;
    let owner = server.account("owner", Role::Owner).await;
    let bob = server.account("bob", Role::Member).await;
    let moder = server.account("moder", Role::Moderator).await;
    let secret = restricted_room(&server, &owner, &[&bob]).await;
    let mut o = Conn::open(&server, &owner).await;
    let (mut b, b_id, _) = Conn::joined(&server, &bob, secret).await;
    let (mut m, m_id, _) = Conn::joined(&server, &moder, secret).await;
    assert!(matches!(b.next().await, ServerMsg::PeerJoined { .. }));

    set_members(&server, &owner, secret, &[]).await;
    assert_eq!(
        b.next().await,
        ServerMsg::PeerLeft {
            user_id: b_id.clone()
        }
    );
    assert!(is_rooms_changed(&b.next().await));
    assert_eq!(m.next().await, ServerMsg::PeerLeft { user_id: b_id });

    let path = format!("/api/users/{m_id}/role");
    let body = json!({ "role": "member" });
    let (status, _, v) = server
        .call(request(
            Method::PATCH,
            &path,
            Some(&body),
            Some(&owner.cookie),
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    m.until(
        "own PeerLeft",
        |x| matches!(x, ServerMsg::PeerLeft { user_id } if *user_id == m_id),
    )
    .await;
    o.until(
        "PeerLeft",
        |x| matches!(x, ServerMsg::PeerLeft { user_id } if *user_id == m_id),
    )
    .await;
    assert!(server.state.voice.peers(|_| true).is_empty());
}

#[tokio::test]
async fn banned_user_cannot_join() {
    let server = TestServer::new().await;
    let owner = server.account("owner", Role::Owner).await;
    let alice = server.account("alice", Role::Member).await;
    let path = format!("/api/users/{}/ban", alice.user.user_id);
    let (status, _, _) = server.call(post(&path, json!({}), &owner)).await;
    assert_eq!(status, StatusCode::OK);
    // As if the socket had been accepted just before the ban.
    let user = server.state.store.user(&alice.user.user_id).await.unwrap();
    let room = rooms(&server).await.remove(0);
    let (tx, _rx) = vesper_server::hub::outbox();
    let client = FakeClient::new().unwrap();
    let r = server
        .state
        .voice
        .join(&user, &room, client.offer().into(), tx)
        .await;
    assert_eq!(r, Err(ErrorCode::Forbidden));
    assert!(server.state.voice.peers(|_| true).is_empty());
}

#[tokio::test]
async fn force_mute_holds_until_lifted() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let moder = server.account("moder", Role::Moderator).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let (mut a, a_id, _) = Conn::joined(&server, &alice, room).await;
    let (mut b, _, _) = Conn::joined(&server, &bob, room).await;
    assert!(matches!(a.next().await, ServerMsg::PeerJoined { .. }));
    let path = format!("/api/users/{a_id}/force-mute");
    let state = |muted, deafened, force_muted| ServerMsg::PeerState {
        user_id: a_id.clone(),
        muted,
        deafened,
        force_muted,
    };

    let (status, _, v) = server
        .call(post(&path, json!({ "muted": true }), &moder))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(a.next().await, ServerMsg::ForceMuted { muted: true });
    assert_eq!(b.next().await, state(true, false, true));
    a.send(&ClientMsg::SetVoiceState {
        muted: false,
        deafened: true,
    });
    assert_eq!(b.next().await, state(true, true, true));

    // Leaving and rejoining does not shake it off.
    a.send(&ClientMsg::Leave);
    let left = ServerMsg::PeerLeft {
        user_id: a_id.clone(),
    };
    assert_eq!(a.next().await, left);
    assert_eq!(b.next().await, left);
    // From a new tab (a fake client cannot negotiate twice).
    let (mut a, _, _) = Conn::joined(&server, &alice, room).await;
    assert_eq!(a.next().await, ServerMsg::ForceMuted { muted: true });
    match b.next().await {
        ServerMsg::PeerJoined { peer } => assert!(peer.muted && peer.force_muted, "{peer:?}"),
        other => panic!("expected PeerJoined, got {other:?}"),
    }

    let (status, _, _) = server
        .call(post(&path, json!({ "muted": false }), &moder))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(a.next().await, ServerMsg::ForceMuted { muted: false });
    // Lifting does not unmute; the user does.
    assert_eq!(b.next().await, state(true, false, false));
    a.mute(false);
    assert_eq!(b.next().await, state(false, false, false));
}

#[tokio::test]
async fn moderator_cannot_kick_owner() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let owner = server.account("owner", Role::Owner).await;
    let moder = server.account("moder", Role::Moderator).await;
    let (_o, o_id, _) = Conn::joined(&server, &owner, room).await;
    let (status, _, _) = server
        .call(post(&format!("/api/users/{o_id}/kick"), json!({}), &moder))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(server.state.voice.peers(|_| true).len(), 1);
}

/// Every caller role against every target role, for all three voice actions. A refusal changes
/// nothing: the target stays in voice, not force-muted, socket open.
#[tokio::test]
async fn voice_actions_check_roles() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let owner = server.account("owner", Role::Owner).await;
    let mod1 = server.account("mod1", Role::Moderator).await;
    let mod2 = server.account("mod2", Role::Moderator).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let mut conns = vec![];
    for s in [&owner, &mod1, &mod2, &alice, &bob] {
        conns.push(Conn::joined(&server, s, room).await.0);
    }
    let actions: [(&str, Value); 3] = [
        ("kick", json!({})),
        ("force-mute", json!({ "muted": true })),
        ("disconnect", json!({})),
    ];
    let untouched = |who: &str| {
        let peers = server.state.voice.peers(|_| true);
        assert!(
            peers.len() == 5 && peers.iter().all(|p| !p.force_muted),
            "after {who}: {peers:?}"
        );
    };
    let refused = [
        (&alice, &bob),
        (&alice, &mod1),
        (&alice, &owner),
        (&alice, &alice),
        (&mod1, &mod2),
        (&mod1, &owner),
        (&mod1, &mod1),
        (&owner, &owner),
    ];
    for (actor, target) in refused {
        for (action, body) in &actions {
            let path = format!("/api/users/{}/{action}", target.user.user_id);
            let (status, _, _) = server.call(post(&path, body.clone(), actor)).await;
            let who = format!("{} {action} {}", actor.user.username, target.user.username);
            assert_eq!(status, StatusCode::FORBIDDEN, "{who}");
            untouched(&who);
        }
    }
    // Unknown id and wrong body.
    let path = format!("/api/users/{}/kick", "f".repeat(32));
    assert_eq!(
        server.call(post(&path, json!({}), &owner)).await.0,
        StatusCode::NOT_FOUND
    );
    let path = format!("/api/users/{}/force-mute", bob.user.user_id);
    let (status, _, _) = server.call(post(&path, json!({}), &owner)).await;
    assert!(status.is_client_error(), "{status}");
    untouched("bad requests");
    // Nobody's socket was closed: everyone still hears a new state.
    conns[4].mute(true);
    for c in &mut conns[..4] {
        c.until_state(&bob.user.user_id).await;
    }

    // Allowed: a moderator on a member, the owner on a moderator.
    for (actor, target) in [(&mod1, &bob), (&owner, &mod2)] {
        let id = &target.user.user_id;
        let call = |action: &str, body: Value| {
            let path = format!("/api/users/{id}/{action}");
            server.call(post(&path, body, actor))
        };
        assert_eq!(
            call("force-mute", json!({ "muted": true })).await.0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(call("kick", json!({})).await.0, StatusCode::NO_CONTENT);
        assert!(
            !server
                .state
                .voice
                .peers(|_| true)
                .iter()
                .any(|p| p.user_id == *id)
        );
        // Not in voice any more.
        assert_eq!(call("kick", json!({})).await.0, StatusCode::NOT_FOUND);
        assert_eq!(
            call("disconnect", json!({})).await.0,
            StatusCode::NO_CONTENT
        );
    }
    let bob_conn = &mut conns[4];
    assert_eq!(bob_conn.expect_closed().await, Some(CLOSE_DISCONNECTED));
    assert_eq!(conns[2].expect_closed().await, Some(CLOSE_DISCONNECTED));
}

#[tokio::test]
async fn disconnect_closes_with_4003_and_leaves_voice() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let moder = server.account("moder", Role::Moderator).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let (mut a, a_id, _) = Conn::joined(&server, &alice, room).await;
    let mut a_chat = Conn::open(&server, &alice).await;
    let (mut b, _, _) = Conn::joined(&server, &bob, room).await;
    let path = format!("/api/users/{a_id}/disconnect");
    let (status, _, _) = server.call(post(&path, json!({}), &moder)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let wait = Duration::from_secs(1);
    assert_eq!(a.expect_closed_within(wait).await, Some(CLOSE_DISCONNECTED));
    assert_eq!(
        a_chat.expect_closed_within(wait).await,
        Some(CLOSE_DISCONNECTED)
    );
    b.until(
        "PeerLeft",
        |m| matches!(m, ServerMsg::PeerLeft { user_id } if *user_id == a_id),
    )
    .await;
    // The session still works: a reconnect is allowed (a ban is what keeps someone out).
    Conn::open(&server, &alice).await;
}

/// The caller's role is read for each call: a demoted moderator is refused, a promoted member
/// is allowed.
#[tokio::test]
async fn voice_actions_read_the_role_per_call() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let owner = server.account("owner", Role::Owner).await;
    let moder = server.account("moder", Role::Moderator).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let (_b, b_id, _) = Conn::joined(&server, &bob, room).await;
    let set_role = async |s: &Session, role: &str| {
        let path = format!("/api/users/{}/role", s.user.user_id);
        let body = json!({ "role": role });
        let req = request(Method::PATCH, &path, Some(&body), Some(&owner.cookie));
        assert_eq!(server.call(req).await.0, StatusCode::OK);
    };
    let kick = format!("/api/users/{b_id}/kick");
    set_role(&moder, "member").await;
    let (status, _, _) = server.call(post(&kick, json!({}), &moder)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(server.state.voice.peers(|_| true).len(), 1);
    set_role(&alice, "moderator").await;
    let (status, _, _) = server.call(post(&kick, json!({}), &alice)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(server.state.voice.peers(|_| true).is_empty());
}

/// `leave` and a closed socket both end the connection check.
#[tokio::test]
async fn check_ends_on_leave_and_on_close() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let voice = &server.state.voice;
    let check = async || {
        let mut c = Conn::open(&server, &alice).await;
        c.send(&ClientMsg::Check {
            offer: c.client.offer().into(),
        });
        assert!(matches!(c.next().await, ServerMsg::Joined { .. }));
        c
    };
    let mut c = check().await;
    assert_eq!(voice.checks(), 1);
    c.send(&ClientMsg::Leave);
    // Barrier: messages are handled in order, and this one fails since no call runs.
    c.mute(true);
    c.expect_error(ErrorCode::BadMessage).await;
    assert_eq!(voice.checks(), 0);

    let c = check().await;
    assert_eq!(voice.checks(), 1);
    drop(c);
    wait_until("check ended on close", || voice.checks() == 0).await;
}

/// The connection check runs on its own SFU session: the user's call in another tab stays up
/// and nobody sees a peer event.
#[tokio::test]
async fn check_does_not_touch_presence_or_own_call() {
    let server = TestServer::new().await;
    let room = general(&server).await;
    let alice = server.account("alice", Role::Member).await;
    let bob = server.account("bob", Role::Member).await;
    let (mut a1, a_id, _) = Conn::joined(&server, &alice, room).await;
    let (b, b_id, _) = Conn::joined(&server, &bob, room).await;
    assert!(matches!(a1.next().await, ServerMsg::PeerJoined { .. }));
    let mut a2 = Conn::open(&server, &alice).await;
    a2.send(&ClientMsg::Check {
        offer: a2.client.offer().into(),
    });
    match a2.next().await {
        ServerMsg::Joined { user_id, peers, .. } => {
            assert!(peers.is_empty(), "{peers:?}");
            assert_ne!(user_id, a_id);
            assert_eq!(user_id.len(), 32);
        }
        other => panic!("expected Joined, got {other:?}"),
    }
    wait_until("check media", || a2.client.is_connected()).await;
    // Barrier: nothing reached a1 between bob's PeerJoined and his mute.
    b.mute(true);
    assert!(a1.until_state(&b_id).await.is_empty());
    assert_eq!(server.state.voice.peers(|_| true).len(), 2);
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
    x.expect_error(ErrorCode::Forbidden).await;
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
async fn reset_link_closes_every_socket_of_that_account() {
    let server = TestServer::new().await;
    let owner = server.owner().await;
    let bob = server.member("bob").await;
    let mut a = Conn::open(&server, &bob).await;
    let mut o = Conn::open(&server, &owner).await;
    let path = format!("/api/users/{}/reset", bob.user.user_id);
    let (status, _, v) = server.call(post(&path, json!({}), &owner)).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let code = v["url"].as_str().unwrap().split_once('#').unwrap().1;
    let body = json!({ "code": code, "password": "test-password-2" });
    let (status, _, v) = server
        .call(request(Method::POST, "/api/reset", Some(&body), None))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(
        a.expect_closed_within(Duration::from_secs(1)).await,
        Some(CLOSE_SESSION_ENDED)
    );
    // The owner's socket stays: it still hears a change.
    let patch = json!({ "display_name": "Boss" });
    let (status, _, _) = server
        .call(request(
            Method::PATCH,
            "/api/me",
            Some(&patch),
            Some(&owner.cookie),
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    o.until(
        "UserUpdated",
        |m| matches!(m, ServerMsg::UserUpdated { user } if user.display_name == "Boss"),
    )
    .await;
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
async fn ban_between_upgrade_and_hub_add_closes_with_4001() {
    let server = TestServer::new().await;
    let owner = server.account("owner", Role::Owner).await;
    let alice = server.account("alice", Role::Member).await;
    // The upgrade passes, then the socket waits before it joins the hub.
    let gate = server.state.hub.hold_adds().await;
    let mut ws = connect(server.addr, Some(&alice.cookie), Some(ORIGIN), false)
        .await
        .expect("upgrade passes before the ban");
    let path = format!("/api/users/{}/ban", alice.user.user_id);
    let (status, _, v) = server.call(post(&path, json!({}), &owner)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    drop(gate);
    // A push right after: it must not reach the dead socket either.
    let (status, _, _) = server
        .call(request(
            Method::PATCH,
            "/api/me",
            Some(&json!({ "display_name": "Owner" })),
            Some(&owner.cookie),
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    match tokio::time::timeout(WAIT, ws.next())
        .await
        .expect("frame in time")
    {
        Some(Ok(Message::Close(Some(f)))) => assert_eq!(u16::from(f.code), CLOSE_SESSION_ENDED),
        other => panic!("expected close 4001 before any message, got {other:?}"),
    }
}

#[tokio::test]
async fn message_flood_is_rate_limited_then_closed() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let mut a = Conn::open(&server, &alice).await;
    // A burst of 20 passes; `typing` gets no reply, so the first reply is the refusal.
    for _ in 0..21 {
        a.send(&ClientMsg::Typing { room_id: 1 });
    }
    a.expect_error(ErrorCode::RateLimited).await;
    for _ in 0..40 {
        a.send(&ClientMsg::Typing { room_id: 1 });
    }
    assert_eq!(
        a.expect_closed().await,
        Some(vesper_server::ws::CLOSE_RATE_LIMITED)
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
    // The close code itself is asserted in the hub unit test. On the wire it is not reliable: a
    // full TCP buffer can keep the close frame from arriving, and the kernel may reset the
    // connection instead (the CI kernel did).
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

/// A signup reaches every open socket as `user_created` (so nobody shows as "Someone"), and a new
/// picture or bio reaches them as `user_updated`.
#[tokio::test]
async fn signup_and_profile_changes_reach_every_socket() {
    let server = TestServer::new().await;
    let alice = server.member("alice").await;
    let owner = server.account("owner", Role::Owner).await;
    let mut a = Conn::open(&server, &alice).await;
    let mut o = Conn::open(&server, &owner).await;
    let bob = server.member("bob").await;
    for c in [&mut a, &mut o] {
        let m = c
            .until("UserCreated", |m| {
                matches!(m, ServerMsg::UserCreated { .. })
            })
            .await;
        let ServerMsg::UserCreated { user } = m else {
            unreachable!()
        };
        assert_eq!(user, bob.user);
    }
    let mut b = Conn::open(&server, &bob).await;
    let (status, _, _) = server
        .call(request(
            Method::PATCH,
            "/api/me",
            Some(&json!({ "bio": "hello" })),
            Some(&bob.cookie),
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    for c in [&mut a, &mut o, &mut b] {
        c.until(
            "UserUpdated",
            |m| matches!(m, ServerMsg::UserUpdated { user } if user.bio == "hello"),
        )
        .await;
    }
    let png = [
        &b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\x01\0\0\0\x01\0\x08\x06\0\0\0\0\0\0\0"[..],
        b"\0\0\0\0IEND\0\0\0\0",
    ]
    .concat();
    let req = axum::http::Request::builder()
        .method(Method::PUT)
        .uri("/api/me/avatar")
        .header("origin", ORIGIN)
        .header("cookie", &bob.cookie)
        .body(axum::body::Body::from(png))
        .unwrap();
    let (status, _, v) = server.call(req).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    a.until(
        "UserUpdated",
        |m| matches!(m, ServerMsg::UserUpdated { user } if user.avatar.is_some()),
    )
    .await;
}
