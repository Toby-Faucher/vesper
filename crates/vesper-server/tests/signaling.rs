//! Signaling end to end: the real server on random ports, a real websocket client per user and
//! `vesper_sfu::testing::FakeClient` playing the browser's WebRTC side.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use vesper_protocol::{ClientMsg, ErrorCode, MAX_MSG_BYTES, MAX_NAME_CHARS, Peer, ServerMsg};
use vesper_server::config::Config;
use vesper_sfu::testing::FakeClient;

/// Generous: CI machines can be slow and ICE/DTLS takes a few round trips.
const WAIT: Duration = Duration::from_secs(10);
const KEY: &str = "test-key";

async fn start(max_room_size: usize) -> SocketAddr {
    let (addr, _serving) = vesper_server::start(Config {
        http_bind: "127.0.0.1:0".parse().unwrap(),
        udp_bind: "127.0.0.1:0".parse().unwrap(),
        public_host: None,
        udp_port_public: 0,
        dev_key: KEY.into(),
        max_room_size,
    })
    .await
    .expect("server starts");
    addr
}

/// One websocket. A background task answers SFU renegotiation offers through the fake client
/// (so sessions are not closed for leaving an offer unanswered) and passes every other message on.
struct Conn {
    client: Arc<FakeClient>,
    out: mpsc::UnboundedSender<Message>,
    msgs: mpsc::UnboundedReceiver<ServerMsg>,
    task: JoinHandle<()>,
}

impl Conn {
    async fn open(addr: SocketAddr) -> Conn {
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .expect("websocket connects");
        let client = Arc::new(FakeClient::new().expect("fake client"));
        let (out, mut out_rx) = mpsc::unbounded_channel::<Message>();
        let (msgs_tx, msgs) = mpsc::unbounded_channel();
        let task = {
            let client = client.clone();
            tokio::spawn(async move {
                let (mut sink, mut stream) = ws.split();
                loop {
                    tokio::select! {
                        m = out_rx.recv() => match m {
                            Some(m) => if sink.send(m).await.is_err() { return },
                            None => return,
                        },
                        m = stream.next() => {
                            let text = match m {
                                Some(Ok(Message::Text(t))) => t,
                                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                                Some(Ok(_)) => continue,
                            };
                            let msg: ServerMsg = serde_json::from_str(&text).expect("valid ServerMsg");
                            match &msg {
                                ServerMsg::Offer { sdp } => {
                                    let sdp = client.accept_offer(sdp).expect("fake client answers");
                                    let a = serde_json::to_string(&ClientMsg::Answer { sdp }).unwrap();
                                    if sink.send(Message::text(a)).await.is_err() { return }
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
        Conn {
            client,
            out,
            msgs,
            task,
        }
    }

    fn send(&self, m: &ClientMsg) {
        self.send_raw(serde_json::to_string(m).unwrap());
    }

    fn send_raw(&self, text: String) {
        self.out
            .send(Message::text(text))
            .expect("connection task alive");
    }

    fn join(&self, room: &str, name: &str, key: &str) {
        self.send(&ClientMsg::Join {
            room: room.into(),
            name: name.into(),
            key: key.into(),
            offer: self.client.offer().into(),
        });
    }

    /// Next non-offer message; panics if none arrives in time or the socket closed.
    async fn next(&mut self) -> ServerMsg {
        tokio::time::timeout(WAIT, self.msgs.recv())
            .await
            .expect("message in time")
            .expect("socket still open")
    }

    /// Join and return (user_id, peers).
    async fn joined(addr: SocketAddr, name: &str) -> (Conn, String, Vec<Peer>) {
        let mut c = Conn::open(addr).await;
        c.join("main", name, KEY);
        match c.next().await {
            ServerMsg::Joined { user_id, peers, .. } => (c, user_id, peers),
            other => panic!("expected Joined, got {other:?}"),
        }
    }

    async fn expect_error(&mut self, code: ErrorCode) {
        match self.next().await {
            ServerMsg::Error { code: c, .. } => assert_eq!(c, code),
            other => panic!("expected Error {code:?}, got {other:?}"),
        }
    }

    /// Wait until the server closes the socket, skipping messages.
    async fn expect_closed(&mut self) {
        let deadline = tokio::time::Instant::now() + WAIT;
        while tokio::time::timeout_at(deadline, self.msgs.recv())
            .await
            .expect("socket closes in time")
            .is_some()
        {}
    }

    /// Messages up to (not including) the `PeerMuted` for `user_id`: a barrier that needs no sleep,
    /// since everything the server queued for this socket earlier arrives first.
    async fn until_muted(&mut self, user_id: &str) -> Vec<ServerMsg> {
        let mut seen = vec![];
        loop {
            match self.next().await {
                ServerMsg::PeerMuted { user_id: u, .. } if u == user_id => return seen,
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

#[tokio::test]
async fn join_returns_answer_and_peer_list() {
    let addr = start(12).await;
    let (mut a, a_id, peers) = Conn::joined(addr, "alice").await;
    assert!(peers.is_empty());
    assert_eq!(a_id.len(), 32);
    assert!(
        a_id.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
        "{a_id}"
    );

    let (b, b_id, peers) = Conn::joined(addr, "bob").await;
    assert_eq!(
        peers,
        vec![Peer {
            user_id: a_id.clone(),
            name: "alice".into(),
            muted: false
        }]
    );
    match a.next().await {
        ServerMsg::PeerJoined { peer } => assert_eq!(
            peer,
            Peer {
                user_id: b_id,
                name: "bob".into(),
                muted: false
            }
        ),
        other => panic!("expected PeerJoined, got {other:?}"),
    }
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
async fn wrong_key_gets_bad_key_error() {
    let addr = start(12).await;
    let mut c = Conn::open(addr).await;
    c.join("main", "mallory", "wrong");
    c.expect_error(ErrorCode::BadKey).await;
    c.join("main", "mallory", "");
    c.expect_error(ErrorCode::BadKey).await;
}

#[tokio::test]
async fn second_session_for_same_user_replaces_first() {
    let addr = start(12).await;
    let (mut a1, a_id, _) = Conn::joined(addr, "alice").await;
    let (mut b, _, _) = Conn::joined(addr, "bob").await;
    assert!(matches!(a1.next().await, ServerMsg::PeerJoined { .. }));

    let (a2, a2_id, peers) = Conn::joined(addr, "alice").await;
    assert_eq!(a2_id, a_id);
    assert_eq!(peers.len(), 1, "only bob: {peers:?}");
    a1.expect_closed().await;

    a2.send(&ClientMsg::SetMuted { muted: true });
    let seen = b.until_muted(&a_id).await;
    assert!(
        matches!(&seen[..], [ServerMsg::PeerLeft { user_id: l }, ServerMsg::PeerJoined { peer }]
            if *l == a_id && peer.user_id == a_id),
        "{seen:?}"
    );

    let (_c, _, peers) = Conn::joined(addr, "carol").await;
    assert_eq!(peers.iter().filter(|p| p.user_id == a_id).count(), 1);
    assert_eq!(peers.len(), 2);
}

#[tokio::test]
async fn room_full_rejects_thirteenth() {
    let addr = start(12).await;
    let mut members = vec![];
    for i in 0..12 {
        let (c, id, peers) = Conn::joined(addr, &format!("user{i}")).await;
        assert_eq!(peers.len(), i);
        members.push((c, id));
    }
    let mut late = Conn::open(addr).await;
    late.join("main", "user12", KEY);
    late.expect_error(ErrorCode::RoomFull).await;

    let (first, first_id) = &members[0];
    first.send(&ClientMsg::SetMuted { muted: true });
    let first_id = first_id.clone();
    for (i, (c, _)) in members.iter_mut().enumerate().skip(1) {
        let seen = c.until_muted(&first_id).await;
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
    let addr = start(12).await;
    let (mut a, _, _) = Conn::joined(addr, "alice").await;

    let mut x = Conn::open(addr).await;
    x.send_raw("this is not json".into());
    x.expect_error(ErrorCode::BadMessage).await;
    x.send_raw(r#"{"type":"nope"}"#.into());
    x.expect_error(ErrorCode::BadMessage).await;
    x.join("main", &"n".repeat(MAX_NAME_CHARS + 1), KEY);
    x.expect_error(ErrorCode::BadMessage).await;
    x.join("main", "   ", KEY);
    x.expect_error(ErrorCode::BadMessage).await;
    x.join("main", "bad\u{7}name", KEY);
    x.expect_error(ErrorCode::BadMessage).await;
    x.send(&ClientMsg::Join {
        room: "main".into(),
        name: "xavier".into(),
        key: KEY.into(),
        offer: "v=0 not an sdp".into(),
    });
    x.expect_error(ErrorCode::BadMessage).await;

    // Still usable after all that.
    x.join("main", &"é".repeat(MAX_NAME_CHARS), KEY);
    let x_id = match x.next().await {
        ServerMsg::Joined { user_id, .. } => user_id,
        other => panic!("expected Joined, got {other:?}"),
    };
    assert!(matches!(a.next().await, ServerMsg::PeerJoined { .. }));

    x.send_raw("x".repeat(MAX_MSG_BYTES + 1));
    x.expect_closed().await;
    match a.next().await {
        ServerMsg::PeerLeft { user_id } => assert_eq!(user_id, x_id),
        other => panic!("expected PeerLeft, got {other:?}"),
    }
    let (_y, _, peers) = Conn::joined(addr, "yuki").await;
    assert_eq!(peers.len(), 1);
}

#[tokio::test]
async fn abrupt_close_removes_presence() {
    let addr = start(12).await;
    let (mut a, _, _) = Conn::joined(addr, "alice").await;
    let (b, b_id, _) = Conn::joined(addr, "bob").await;
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
async fn resolve_public_returns_loopback() {
    let addrs = vesper_server::public_addr::resolve_public("localhost", 40000).await;
    assert!(
        addrs
            .iter()
            .any(|a| a.ip().is_loopback() && a.port() == 40000),
        "{addrs:?}"
    );
}

#[tokio::test]
async fn health_returns_ok() {
    let addr = start(12).await;
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut resp = String::new();
    s.read_to_string(&mut resp).await.unwrap();
    assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
    assert!(resp.ends_with("\r\n\r\nok"), "{resp}");
}
