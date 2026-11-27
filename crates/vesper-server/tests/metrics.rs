//! `/metrics` end to end: the real server with `metrics_bind` set, scraped over HTTP like VictoriaMetrics or
//! Prometheus would, while a signed-in socket is in a call.

mod common;

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::Duration;

use common::{ORIGIN, TestServer, spawn_sfu};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use vesper_protocol::{ClientMsg, Role, ServerMsg};
use vesper_sfu::testing::FakeClient;

const WAIT: Duration = Duration::from_secs(10);

/// A free loopback port. Another process could take it before the server binds it; on a test machine that is rare.
fn free_port() -> SocketAddr {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap()
}

/// `GET path` over plain HTTP/1.1. Returns the head and the body.
async fn http_get(addr: SocketAddr, path: &str) -> (String, String) {
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    tcp.write_all(req.as_bytes()).await.unwrap();
    let mut raw = String::new();
    tokio::time::timeout(WAIT, tcp.read_to_string(&mut raw))
        .await
        .expect("answer in time")
        .unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").expect("an HTTP answer");
    (head.to_string(), body.to_string())
}

/// Every sample in the exposition, keyed by its name and labels as written. Checks the text format on the way:
/// each sample's family has a `# TYPE` line before it, and no series appears twice.
fn samples(body: &str) -> HashMap<String, f64> {
    let mut typed = HashSet::new();
    let mut out = HashMap::new();
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("# TYPE ") {
            let (name, kind) = rest.split_once(' ').unwrap();
            assert!(matches!(kind, "counter" | "gauge"), "{line}");
            assert!(typed.insert(name.to_string()), "family typed twice: {line}");
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let (series, value) = line.rsplit_once(' ').unwrap();
        let name = series.split('{').next().unwrap();
        assert!(typed.contains(name), "sample before its TYPE: {line}");
        let value: f64 = value
            .parse()
            .unwrap_or_else(|_| panic!("bad value: {line}"));
        assert!(
            out.insert(series.to_string(), value).is_none(),
            "series twice: {line}"
        );
    }
    out
}

async fn scrape(addr: SocketAddr) -> HashMap<String, f64> {
    let (head, body) = http_get(addr, "/metrics").await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(
        head.to_ascii_lowercase()
            .contains("content-type: text/plain; version=0.0.4; charset=utf-8"),
        "{head}"
    );
    samples(&body)
}

fn value(m: &HashMap<String, f64>, series: &str) -> f64 {
    *m.get(series)
        .unwrap_or_else(|| panic!("no {series} in {:#?}", m.keys().collect::<Vec<_>>()))
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn upgrade(
    addr: SocketAddr,
    cookie: Option<&str>,
    origin: &str,
) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    let mut req = format!("ws://{addr}/ws").into_client_request().unwrap();
    if let Some(c) = cookie {
        req.headers_mut().insert("cookie", c.parse().unwrap());
    }
    req.headers_mut().insert("origin", origin.parse().unwrap());
    tokio_tungstenite::connect_async(req)
        .await
        .map(|(ws, _)| ws)
}

/// The next server message that `f` picks, skipping the rest.
async fn until(ws: &mut Ws, what: &str, f: impl Fn(&ServerMsg) -> bool) -> ServerMsg {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let m = tokio::time::timeout_at(deadline, ws.next())
            .await
            .unwrap_or_else(|_| panic!("no {what} in time"))
            .unwrap_or_else(|| panic!("socket closed before {what}"))
            .unwrap();
        if let Message::Text(t) = m {
            let msg: ServerMsg = serde_json::from_str(&t).unwrap();
            if f(&msg) {
                return msg;
            }
        }
    }
}

#[tokio::test]
async fn metrics_carry_the_key_series_on_their_own_port() {
    let metrics = free_port();
    let server = TestServer::with(|c| c.metrics_bind = Some(metrics), spawn_sfu()).await;

    let m = scrape(metrics).await;
    let build = format!(
        "vesper_build_info{{version=\"{}\",build=\"test-build\"}}",
        vesper_server::VERSION
    );
    assert_eq!(value(&m, &build), 1.0);
    assert!(value(&m, "process_start_time_seconds") > 1.7e9);
    assert_eq!(value(&m, "vesper_sfu_up"), 1.0);
    for g in [
        "vesper_ws_connections",
        "vesper_voice_participants",
        "vesper_voice_rooms",
        "vesper_sfu_sessions",
        "vesper_sfu_rooms",
        "vesper_push_pending",
    ] {
        assert_eq!(value(&m, g), 0.0, "{g}");
    }
    for dir in ["in", "out"] {
        for kind in ["audio", "video", "rtcp", "other"] {
            value(
                &m,
                &format!("vesper_sfu_packets_total{{direction=\"{dir}\",kind=\"{kind}\"}}"),
            );
        }
    }
    value(
        &m,
        "vesper_sfu_packets_dropped_total{direction=\"in\",kind=\"other\",reason=\"no_session\"}",
    );
    value(
        &m,
        "vesper_sfu_media_dropped_total{kind=\"audio\",reason=\"muted\"}",
    );
    value(
        &m,
        "vesper_sfu_media_dropped_total{kind=\"video\",reason=\"not_sharing\"}",
    );
    value(&m, "vesper_push_sends_total{result=\"failed\"}");
    // The media socket is in /proc/net/udp on Linux.
    if cfg!(target_os = "linux") {
        assert_eq!(value(&m, "vesper_sfu_socket_receive_drops_total"), 0.0);
        value(&m, "vesper_sfu_socket_receive_queue_bytes");
    }

    // The app's own port does not serve it.
    let (head, body) = http_get(server.addr, "/metrics").await;
    assert!(head.starts_with("HTTP/1.1 404"), "{head}");
    assert!(!body.contains("vesper_build_info"));

    // Refused upgrades are counted by reason.
    let s = server.account("mel", Role::Member).await;
    let other = upgrade(server.addr, Some(&s.cookie), "http://evil.example").await;
    assert!(
        matches!(other, Err(tokio_tungstenite::tungstenite::Error::Http(r)) if r.status() == 403)
    );
    let anon = upgrade(server.addr, None, ORIGIN).await;
    assert!(
        matches!(anon, Err(tokio_tungstenite::tungstenite::Error::Http(r)) if r.status() == 401)
    );

    // A signed-in socket in a call.
    let mut ws = upgrade(server.addr, Some(&s.cookie), ORIGIN).await.unwrap();
    until(&mut ws, "Hello", |m| matches!(m, ServerMsg::Hello { .. })).await;
    let rooms = server.state.store.rooms().await.unwrap();
    let general = rooms.iter().find(|r| r.name == "general").unwrap().id;
    let client = FakeClient::new().unwrap();
    let join = ClientMsg::Join {
        room_id: general,
        offer: client.offer().into(),
    };
    ws.send(Message::text(serde_json::to_string(&join).unwrap()))
        .await
        .unwrap();
    until(&mut ws, "Joined", |m| matches!(m, ServerMsg::Joined { .. })).await;

    let m = scrape(metrics).await;
    assert_eq!(value(&m, "vesper_ws_connections"), 1.0);
    assert_eq!(value(&m, "vesper_ws_connections_opened_total"), 1.0);
    assert_eq!(value(&m, "vesper_ws_refused_total{reason=\"origin\"}"), 1.0);
    assert_eq!(value(&m, "vesper_ws_refused_total{reason=\"auth\"}"), 1.0);
    assert_eq!(value(&m, "vesper_ws_refused_total{reason=\"full\"}"), 0.0);
    assert_eq!(
        value(&m, "vesper_ws_closed_total{reason=\"read_failed\"}"),
        0.0
    );
    assert_eq!(value(&m, "vesper_voice_participants"), 1.0);
    assert_eq!(value(&m, "vesper_voice_rooms"), 1.0);
    assert_eq!(value(&m, "vesper_sfu_sessions"), 1.0);
    assert_eq!(value(&m, "vesper_sfu_rooms"), 1.0);
    // No user, room or session id in any series.
    let (_, body) = http_get(metrics, "/metrics").await;
    assert!(
        !body.contains(&s.user.user_id),
        "a user id leaked into /metrics"
    );
    assert!(
        !body.contains("general"),
        "a room name leaked into /metrics"
    );
}
