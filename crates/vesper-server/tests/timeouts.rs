//! Header and body timeouts over raw TCP, with short timers from the test config: a stalled head,
//! a stalled or trickled JSON body and a kept-alive idle connection are closed; a slow upload, a
//! WebSocket and an aborted upload's partial file behave.

mod common;

use std::time::Duration;

use common::{ORIGIN, Session, TestServer, spawn_sfu};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use vesper_protocol::Role;

const HEADER: Duration = Duration::from_secs(1);
const IDLE: Duration = Duration::from_millis(500);
const TOTAL: Duration = Duration::from_secs(3);
/// How long a test waits for a close: the timer plus slack for a slow CI machine. Less than
/// `TOTAL - IDLE`, so the idle test fails when only the total timer is left.
const SLACK: Duration = Duration::from_millis(1500);

async fn server() -> TestServer {
    TestServer::with(
        |c| {
            c.header_timeout = HEADER;
            c.body_idle = IDLE;
            c.body_total = TOTAL;
        },
        spawn_sfu(),
    )
    .await
}

/// Reads until the server closes; returns what it sent, or panics if it is still open after `wait`.
async fn read_until_closed(t: &mut TcpStream, wait: Duration) -> String {
    let mut b = Vec::new();
    match tokio::time::timeout(wait, t.read_to_end(&mut b)).await {
        Ok(_) => String::from_utf8_lossy(&b).into_owned(),
        Err(_) => panic!(
            "still open after {wait:?}; got {:?}",
            String::from_utf8_lossy(&b)
        ),
    }
}

#[tokio::test]
async fn half_a_request_head_is_closed() {
    let server = server().await;
    let mut t = TcpStream::connect(server.addr).await.unwrap();
    t.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\n")
        .await
        .unwrap();
    read_until_closed(&mut t, HEADER + SLACK).await;
}

#[tokio::test]
async fn idle_kept_alive_connection_is_closed() {
    let server = server().await;
    let mut t = TcpStream::connect(server.addr).await.unwrap();
    t.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let res = read_until_closed(&mut t, HEADER + SLACK).await;
    assert!(res.starts_with("HTTP/1.1 200"), "{res}");
}

fn login_head(len: usize) -> String {
    format!(
        "POST /api/login HTTP/1.1\r\nHost: x\r\nOrigin: {ORIGIN}\r\n\
         Content-Type: application/json\r\nContent-Length: {len}\r\n\r\n"
    )
}

#[tokio::test]
async fn stalled_json_body_is_closed() {
    let server = server().await;
    let mut t = TcpStream::connect(server.addr).await.unwrap();
    t.write_all(login_head(100).as_bytes()).await.unwrap();
    t.write_all(b"{\"user").await.unwrap();
    read_until_closed(&mut t, IDLE + SLACK).await;
}

#[tokio::test]
async fn trickled_json_body_is_closed_at_the_total() {
    let server = server().await;
    let mut t = TcpStream::connect(server.addr).await.unwrap();
    t.write_all(login_head(100).as_bytes()).await.unwrap();
    let (mut rd, mut wr) = t.into_split();
    // One byte every 200 ms never trips the idle timer; 100 bytes would take 20 s.
    let trickle = tokio::spawn(async move {
        for _ in 0..100 {
            if wr.write_all(b" ").await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    });
    let mut b = Vec::new();
    let read = tokio::time::timeout(TOTAL + SLACK, rd.read_to_end(&mut b)).await;
    trickle.abort();
    assert!(read.is_ok(), "still open after {:?}", TOTAL + SLACK);
}

fn upload_head(s: &Session, room: i64, len: usize) -> String {
    format!(
        "POST /api/rooms/{room}/uploads?name=slow.bin HTTP/1.1\r\nHost: x\r\nOrigin: {ORIGIN}\r\n\
         Cookie: {}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n",
        s.cookie
    )
}

#[tokio::test]
async fn slow_upload_outlasts_the_json_total() {
    let server = server().await;
    let alice = server.account("alice", Role::Member).await;
    let room = server.state.store.rooms().await.unwrap()[0].id;
    let mut t = TcpStream::connect(server.addr).await.unwrap();
    t.write_all(upload_head(&alice, room, 42).as_bytes())
        .await
        .unwrap();
    // 14 chunks 300 ms apart: 4.2 s in all, past the 3 s JSON total and the 500 ms JSON idle.
    for _ in 0..14 {
        tokio::time::sleep(Duration::from_millis(300)).await;
        t.write_all(b"abc").await.unwrap();
    }
    let res = read_until_closed(&mut t, SLACK).await;
    assert!(res.starts_with("HTTP/1.1 201"), "{res}");
}

#[tokio::test]
async fn aborted_upload_leaves_no_partial_file() {
    let server = server().await;
    let alice = server.account("alice", Role::Member).await;
    let room = server.state.store.rooms().await.unwrap()[0].id;
    let tmp = server.data.path().join("uploads").join("tmp");
    let count = || std::fs::read_dir(&tmp).map_or(0, |d| d.count());
    let mut t = TcpStream::connect(server.addr).await.unwrap();
    t.write_all(upload_head(&alice, room, 1000).as_bytes())
        .await
        .unwrap();
    t.write_all(&[7u8; 100]).await.unwrap();
    let until = |want: usize| async move {
        let deadline = tokio::time::Instant::now() + SLACK;
        while count() != want {
            assert!(
                tokio::time::Instant::now() < deadline,
                "uploads/tmp never had {want} files"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    until(1).await;
    drop(t);
    until(0).await;
    assert!(
        server.state.store.upload_bytes().await.unwrap() == 0,
        "no upload row is kept"
    );
}

#[tokio::test]
async fn websocket_outlives_every_timeout() {
    let server = server().await;
    let alice = server.account("alice", Role::Member).await;
    let mut req = format!("ws://{}/ws", server.addr)
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("cookie", alice.cookie.parse().unwrap());
    req.headers_mut().insert("origin", ORIGIN.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    // Past the header, idle and total timers together.
    tokio::time::sleep(HEADER + TOTAL + Duration::from_secs(1)).await;
    ws.send(Message::Ping(b"still here".to_vec().into()))
        .await
        .unwrap();
    let pong = tokio::time::timeout(SLACK, async {
        while let Some(m) = ws.next().await {
            if let Message::Pong(p) = m.unwrap() {
                return p;
            }
        }
        panic!("socket closed");
    })
    .await
    .expect("a pong in time");
    assert_eq!(&pong[..], b"still here");
}
