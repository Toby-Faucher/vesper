//! `/metrics` in the Prometheus text format (version 0.0.4), on a listener of its own (`VESPER_METRICS_BIND`), off
//! by default. It has no auth and no Origin check, so it must not be reachable from the internet: Caddy only proxies
//! `VESPER_HTTP_BIND`, and this is another port. Labels are fixed sets (kind, reason, direction); nothing is per user
//! or per room. Every value is read when scraped: atomics, two locks held for a count, and `/proc/net/udp`.

use std::fmt::{Display, Write as _};
use std::net::SocketAddr;

use axum::Router;
use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use tokio::task::JoinHandle;
use vesper_sfu::{MediaDrop, MediaKind, PacketKind, SfuHandle, SfuStats};

use crate::ws::{End, Refused, WsState};

/// What a scrape reads.
#[derive(Clone)]
pub struct MetricsState {
    pub ws: WsState,
    pub sfu: SfuHandle,
}

/// Bind `bind` and serve `GET /metrics` there. Returns the bound address and the serving task.
pub async fn serve(
    bind: SocketAddr,
    state: MetricsState,
) -> std::io::Result<(SocketAddr, JoinHandle<std::io::Result<()>>)> {
    let header_timeout = state.ws.app.cfg.header_timeout;
    let app = Router::new()
        .route("/metrics", get(metrics))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| std::io::Error::other(format!("VESPER_METRICS_BIND={bind}: {e}")))?;
    let addr = listener.local_addr()?;
    Ok((
        addr,
        tokio::spawn(crate::accept_loop(listener, app, header_timeout)),
    ))
}

async fn metrics(State(st): State<MetricsState>) -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        render(&st),
    )
}

/// The whole exposition.
pub fn render(st: &MetricsState) -> String {
    let mut o = Out(String::with_capacity(8 * 1024));
    let app = &st.ws.app;

    let b = app.cfg.server_build();
    o.family(
        "vesper_build_info",
        "gauge",
        "Always 1; the labels say which release and build runs.",
    );
    o.sample(
        "vesper_build_info",
        &[("version", &b.version), ("build", &b.build)],
        1,
    );
    if let Some(started) = crate::STARTED.get() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let start = now.saturating_sub(started.elapsed()).as_secs_f64();
        o.family(
            "process_start_time_seconds",
            "gauge",
            "When the server started, in Unix seconds.",
        );
        o.sample("process_start_time_seconds", &[], format!("{start:.3}"));
    }

    // Voice, as the server sees it.
    let v = app.voice.stats();
    o.gauge(
        "vesper_voice_participants",
        "People in a call, those whose socket is away included.",
        v.participants,
    );
    o.gauge(
        "vesper_voice_rooms",
        "Rooms with at least one person in voice.",
        v.rooms,
    );
    o.gauge(
        "vesper_voice_away",
        "People whose socket dropped and whose call waits for a resume.",
        v.away,
    );
    o.gauge(
        "vesper_voice_sharing",
        "People sharing their screen.",
        v.sharing,
    );
    o.gauge(
        "vesper_voice_checks",
        "Connection checks running.",
        v.checks,
    );

    // The SFU.
    o.gauge(
        "vesper_sfu_up",
        "1 while the SFU thread runs, 0 once it died.",
        u64::from(st.sfu.is_alive()),
    );
    let s = st.sfu.stats();
    sfu(&mut o, &s);
    if let Some(q) = udp_socket(st.sfu.local_addr().port()) {
        o.family(
            "vesper_sfu_socket_receive_drops_total",
            "counter",
            "Datagrams the kernel dropped because the media socket's receive buffer was full (/proc/net/udp).",
        );
        o.sample("vesper_sfu_socket_receive_drops_total", &[], q.drops);
        o.gauge(
            "vesper_sfu_socket_receive_queue_bytes",
            "Bytes waiting in the media socket's receive buffer (/proc/net/udp).",
            q.rx_queue,
        );
    }
    // Keyframe requests forwarded and merged come with the PLI rate limit (PR #208, `SfuHandle::keyframe_stats`).

    // WebSockets.
    let w = &st.ws.stats;
    o.gauge(
        "vesper_ws_connections",
        "Signaling WebSockets open now.",
        w.open(),
    );
    o.family(
        "vesper_ws_connections_opened_total",
        "counter",
        "Signaling WebSockets opened since start.",
    );
    o.sample("vesper_ws_connections_opened_total", &[], w.opened());
    o.family(
        "vesper_ws_refused_total",
        "counter",
        "WebSocket upgrades refused, by reason.",
    );
    for r in Refused::ALL {
        o.sample(
            "vesper_ws_refused_total",
            &[("reason", r.label())],
            w.refused(r),
        );
    }
    o.family(
        "vesper_ws_closed_total",
        "counter",
        "Open WebSockets that ended, by reason. read_failed, write_failed and ping_failed are errors.",
    );
    for e in End::ALL {
        o.sample(
            "vesper_ws_closed_total",
            &[("reason", e.label())],
            w.ended(e),
        );
    }

    // Web Push.
    let p = app.push.counts();
    o.family(
        "vesper_push_sends_total",
        "counter",
        "Web Push sends, by result.",
    );
    for (result, n) in [("ok", p.ok), ("gone", p.gone), ("failed", p.failed)] {
        o.sample("vesper_push_sends_total", &[("result", result)], n);
    }
    o.family(
        "vesper_push_dropped_total",
        "counter",
        "Pushes never sent because the queue was full.",
    );
    o.sample("vesper_push_dropped_total", &[], p.dropped);
    o.gauge(
        "vesper_push_pending",
        "Pushes waiting for a slot or in flight.",
        app.push.pending() as u64,
    );
    o.0
}

fn sfu(o: &mut Out, s: &SfuStats) {
    o.gauge(
        "vesper_sfu_sessions",
        "Live SFU peer connections, connection checks included.",
        s.sessions,
    );
    o.gauge(
        "vesper_sfu_rooms",
        "Rooms with at least one live SFU session; a connection check is its own room.",
        s.rooms,
    );
    o.family(
        "vesper_sfu_packets_total",
        "counter",
        "Datagrams on the media socket, by direction and kind. out counts every send tried.",
    );
    for (dir, by) in [("in", &s.packets_in), ("out", &s.packets_out)] {
        for k in PacketKind::ALL {
            o.sample(
                "vesper_sfu_packets_total",
                &[("direction", dir), ("kind", k.label())],
                by.get(k),
            );
        }
    }
    o.family(
        "vesper_sfu_packets_dropped_total",
        "counter",
        "Datagrams dropped: in with no session to take them, out whose send failed.",
    );
    for (dir, reason, by) in [
        ("in", "no_session", &s.dropped_no_session),
        ("out", "send_error", &s.dropped_send_error),
    ] {
        for k in PacketKind::ALL {
            let labels = [("direction", dir), ("kind", k.label()), ("reason", reason)];
            o.sample("vesper_sfu_packets_dropped_total", &labels, by.get(k));
        }
    }
    o.family(
        "vesper_sfu_media_dropped_total",
        "counter",
        "Media the SFU received and did not forward, by reason. Audio counts packets, video counts frames.",
    );
    for why in MediaDrop::ALL {
        for (kind, label) in [(MediaKind::Audio, "audio"), (MediaKind::Video, "video")] {
            let labels = [("kind", label), ("reason", why.label())];
            o.sample(
                "vesper_sfu_media_dropped_total",
                &labels,
                s.media_dropped(kind, why),
            );
        }
    }
}

/// The text being built.
struct Out(String);

impl Out {
    fn family(&mut self, name: &str, kind: &str, help: &str) {
        let _ = writeln!(self.0, "# HELP {name} {help}\n# TYPE {name} {kind}");
    }

    fn gauge(&mut self, name: &str, help: &str, value: u64) {
        self.family(name, "gauge", help);
        self.sample(name, &[], value);
    }

    fn sample(&mut self, name: &str, labels: &[(&str, &str)], value: impl Display) {
        self.0.push_str(name);
        if !labels.is_empty() {
            self.0.push('{');
            for (i, (k, v)) in labels.iter().enumerate() {
                if i > 0 {
                    self.0.push(',');
                }
                let _ = write!(self.0, "{k}=\"{}\"", escape(v));
            }
            self.0.push('}');
        }
        let _ = writeln!(self.0, " {value}");
    }
}

/// A label value: backslash, double quote and newline escaped.
fn escape(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// The kernel's view of one UDP socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpQueue {
    /// Bytes in the receive buffer.
    pub rx_queue: u64,
    /// Datagrams dropped since the socket opened (a full buffer, mostly).
    pub drops: u64,
}

/// The media socket in `/proc/net/udp` or `/proc/net/udp6`, found by its local port. None off Linux, or when
/// `/proc` cannot be read. Read here, not in the SFU's receive loop, so the receive path stays as it is.
fn udp_socket(port: u16) -> Option<UdpQueue> {
    ["/proc/net/udp", "/proc/net/udp6"]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .find_map(|t| parse_proc_udp(&t, port))
}

/// The first socket with local port `port` in a `/proc/net/udp` table: `sl local rem st tx:rx tr retrnsmt uid
/// timeout inode ref pointer drops`, addresses and queues in hex.
pub fn parse_proc_udp(table: &str, port: u16) -> Option<UdpQueue> {
    table.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        let local_port = f.get(1)?.rsplit_once(':')?.1;
        if u16::from_str_radix(local_port, 16).ok()? != port {
            return None;
        }
        let rx_queue = u64::from_str_radix(f.get(4)?.split_once(':')?.1, 16).ok()?;
        let drops = f.get(12)?.parse().ok()?;
        Some(UdpQueue { rx_queue, drops })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "   sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode ref pointer drops
 1594: 0100007F:88CA 00000000:0000 07 00000000:00000000 00:00000000 00000000  1001        0 2581245 2 0000000000000000 0
 1878: 00000000:9C40 00000000:0000 07 00000000:00000A00 00:00000000 00000000 10001        0 2577338 2 0000000000000000 17
";

    #[test]
    fn the_media_socket_is_found_by_port() {
        assert_eq!(
            parse_proc_udp(TABLE, 40000),
            Some(UdpQueue {
                rx_queue: 2560,
                drops: 17
            })
        );
        assert_eq!(
            parse_proc_udp(TABLE, 0x88ca),
            Some(UdpQueue {
                rx_queue: 0,
                drops: 0
            })
        );
        assert_eq!(parse_proc_udp(TABLE, 40001), None);
        assert_eq!(parse_proc_udp("", 40000), None);
        assert_eq!(parse_proc_udp("header\n 1: garbage", 40000), None);
    }

    #[test]
    fn samples_follow_the_text_format() {
        let mut o = Out(String::new());
        o.family("x_total", "counter", "An example.");
        o.sample("x_total", &[], 3);
        o.sample("x_total", &[("kind", "a\"b\\c\nd"), ("reason", "r")], 4);
        assert_eq!(
            o.0,
            "# HELP x_total An example.\n# TYPE x_total counter\nx_total 3\n\
             x_total{kind=\"a\\\"b\\\\c\\nd\",reason=\"r\"} 4\n"
        );
    }
}
