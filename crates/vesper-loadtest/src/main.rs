#![forbid(unsafe_code)]

//! Synthetic voice load: N fake browsers join one room over the real WebSocket protocol, each
//! sends 20 ms Opus-sized packets, and every speaker counts what it gets from each other speaker.
//! It also times every forwarded packet (all fake clients share one clock), and with `--pid` reads
//! the server's CPU and memory. `--json FILE` writes the numbers for `scripts/loadtest-report.py`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use vesper_protocol::{ClientMsg, ServerMsg};
use vesper_sfu::testing::{FakeClient, MediaKind};

const PASS_PERCENT: f64 = 95.0;
const PACKET_MS: u64 = 20;
const VIDEO_FPS: u64 = 60;

/// Per-speaker verdict.
#[derive(Debug, PartialEq)]
struct SpeakerReport {
    /// Packets received from the other speakers, each peer clamped to what it sent.
    received: u64,
    /// Packets the other speakers sent to this one.
    expected: u64,
    loss_percent: f64,
    /// Worst peer under the bar: (sender index, percent received from it).
    bad_pair: Option<(usize, f64)>,
    ok: bool,
}

/// `sent[j]` = packets speaker j sent; `received[i][j]` = packets speaker i got from speaker j
/// (`received[i][i]` is ignored). A speaker passes if it got at least 95% of its peers' packets;
/// with no peers (a single speaker, kept as a documented trivial pass) it passes trivially.
/// It must ALSO get 95% from every individual peer, so one dead peer fails at any room size.
/// Counts above what a peer sent are clamped.
fn summarize(sent: &[u64], received: &[Vec<u64>]) -> Vec<SpeakerReport> {
    (0..sent.len())
        .map(|i| {
            let (mut got, mut expected) = (0, 0);
            let mut bad_pair: Option<(usize, f64)> = None;
            for (j, &s) in sent.iter().enumerate().filter(|&(j, _)| j != i) {
                let g = received[i].get(j).copied().unwrap_or(0).min(s);
                got += g;
                expected += s;
                let p = if s == 0 {
                    0.0
                } else {
                    g as f64 * 100.0 / s as f64
                };
                if p < PASS_PERCENT && bad_pair.is_none_or(|(_, w)| p < w) {
                    bad_pair = Some((j, p));
                }
            }
            let has_peers = sent.len() > 1;
            let pct = if expected == 0 {
                0.0
            } else {
                got as f64 * 100.0 / expected as f64
            };
            SpeakerReport {
                received: got,
                expected,
                loss_percent: if has_peers { 100.0 - pct } else { 0.0 },
                bad_pair,
                // Peers that sent nothing means the test itself is broken: fail.
                ok: !has_peers || (pct >= PASS_PERCENT && bad_pair.is_none()),
            }
        })
        .collect()
}

struct Args {
    url: String,
    /// Sent as `Origin` on every upgrade; also picks the cookie name.
    origin: String,
    /// One session token per speaker, from `vesper-server loadtest-accounts`.
    tokens: Vec<String>,
    room_id: i64,
    speakers: usize,
    /// With sharers, every speaker is built `with_video` (a viewer needs VP8). The first `sharers` speakers share a screen; everyone watches them.
    sharers: usize,
    share_mbps: u64,
    seconds: u64,
    /// Gap between one speaker's audio packets, 1 to 20 ms (20 is real Opus; less is a throughput test).
    packet_ms: u64,
    /// After the run, drop this many speakers' sockets and time their `resume` (0 to skip).
    reconnects: usize,
    pid: Option<u32>,
    /// Write a summary of the run here as JSON.
    json: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut m: HashMap<String, String> = HashMap::new();
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let v = it.next().ok_or(format!("{flag} needs a value"))?;
        m.insert(flag, v);
    }
    let get = |k: &str| m.get(k).cloned().ok_or(format!("missing {k}"));
    let num = |k: &str| get(k)?.parse::<u64>().map_err(|e| format!("{k}: {e}"));
    let (speakers, seconds) = (num("--speakers")? as usize, num("--seconds")?);
    if speakers < 1 {
        return Err("--speakers must be at least 1 (1 is a trivial pass: no peers)".into());
    }
    if seconds < 1 {
        return Err("--seconds must be at least 1".into());
    }
    let opt = |k: &str, d: u64| match m.get(k) {
        Some(v) => v.parse::<u64>().map_err(|e| format!("{k}: {e}")),
        None => Ok(d),
    };
    let (sharers, share_mbps) = (opt("--sharers", 0)? as usize, opt("--share-mbps", 10)?);
    let packet_ms = opt("--packet-ms", PACKET_MS)?;
    if !(1..=PACKET_MS).contains(&packet_ms) {
        return Err(format!("--packet-ms must be 1 to {PACKET_MS}"));
    }
    if sharers > speakers {
        return Err("--sharers cannot exceed --speakers".into());
    }
    if !(1..=30).contains(&share_mbps) {
        return Err(
            "--share-mbps must be 1 to 30 (the most the server accepts for a share)".into(),
        );
    }
    let file = get("--tokens")?;
    let tokens: Vec<String> = std::fs::read_to_string(&file)
        .map_err(|e| format!("--tokens {file}: {e}"))?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    if tokens.len() < speakers {
        return Err(format!(
            "--tokens {file}: {} tokens for {speakers} speakers",
            tokens.len()
        ));
    }
    Ok(Args {
        url: get("--url")?,
        origin: get("--origin")?,
        tokens,
        room_id: get("--room-id")?
            .parse()
            .map_err(|e| format!("--room-id: {e}"))?,
        speakers,
        sharers,
        share_mbps,
        seconds,
        packet_ms,
        reconnects: opt("--reconnects", 0)? as usize,
        pid: match m.get("--pid") {
            Some(p) => Some(p.parse().map_err(|e| format!("--pid: {e}"))?),
            None => None,
        },
        json: m.get("--json").cloned(),
    })
}

/// utime + stime in clock ticks from /proc/<pid>/stat (Linux only).
fn cpu_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The comm field may contain spaces; fields resume after the last ')'. utime/stime are fields 14/15.
    let rest: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    Some(rest.get(11)?.parse::<u64>().ok()? + rest.get(12)?.parse::<u64>().ok()?)
}

/// A `kB` field such as `VmRSS` from /proc/<pid>/status, in MiB (Linux only).
fn status_mib(pid: u32, field: &str) -> Option<f64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|l| l.starts_with(field))?;
    let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

/// The value at fraction `q` (0..=1) of sorted `v`, nearest rank. 0 for an empty slice.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// Median of an unsorted slice; 0 when empty.
fn median(v: &[f64]) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    match v.len() {
        0 => 0.0,
        n if n % 2 == 1 => v[n / 2],
        n => (v[n / 2 - 1] + v[n / 2]) / 2.0,
    }
}

/// RFC 3550 section 6.4.1 interarrival jitter over one stream's transit times (arrival minus send,
/// in arrival order), in the same unit: J += (|D| - J) / 16 for each consecutive pair.
fn rfc3550_jitter(transits: &[f64]) -> f64 {
    transits
        .windows(2)
        .fold(0.0, |j, w| j + ((w[1] - w[0]).abs() - j) / 16.0)
}

/// Forwarding times of every audio packet that arrived, and the jitter of each receiver-sender pair.
#[derive(Debug, Default, PartialEq)]
struct Timing {
    /// Send to arrival, ms, sorted.
    latency_ms: Vec<f64>,
    /// RFC 3550 jitter of each receiver-sender pair with at least two packets, ms.
    pair_jitter_ms: Vec<f64>,
}

/// `sent[j]` maps speaker j's packet sequence number to when it was written; `arrivals[i]` lists
/// (sender, sequence number, arrival time) as speaker i saw them, in arrival order.
fn timing(sent: &[HashMap<u64, Instant>], arrivals: &[Vec<(usize, u64, Instant)>]) -> Timing {
    let mut t = Timing::default();
    for row in arrivals {
        let mut per_sender: HashMap<usize, Vec<f64>> = HashMap::new();
        for &(j, seq, at) in row {
            let Some(&sent_at) = sent.get(j).and_then(|m| m.get(&seq)) else {
                continue;
            };
            let ms = at.saturating_duration_since(sent_at).as_secs_f64() * 1000.0;
            t.latency_ms.push(ms);
            per_sender.entry(j).or_default().push(ms);
        }
        let mut senders: Vec<_> = per_sender.into_iter().collect();
        senders.sort_by_key(|(j, _)| *j);
        t.pair_jitter_ms.extend(
            senders
                .iter()
                .filter(|(_, v)| v.len() > 1)
                .map(|(_, v)| rfc3550_jitter(v)),
        );
    }
    t.latency_ms.sort_by(f64::total_cmp);
    t
}

/// shortcut: assumes CLK_TCK = 100 (true on Linux x86/ARM); reading it needs libc/unsafe.
const CLK_TCK: f64 = 100.0;

type Sink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

struct Speaker {
    client: Arc<FakeClient>,
    task: JoinHandle<()>,
    sink: Arc<tokio::sync::Mutex<Sink>>,
    user_id: String,
    /// This call's number for `resume`; 0 from a server without resume (before v2.3.2).
    session: u64,
    /// From starting the WebSocket connect to `joined`.
    join_ms: f64,
}

impl Speaker {
    async fn send(&self, m: &ClientMsg) -> Result<(), String> {
        let text = serde_json::to_string(m).unwrap();
        let mut sink = self.sink.lock().await;
        sink.send(Message::text(text))
            .await
            .map_err(|e| e.to_string())
    }
}

/// Open speaker `n`'s WebSocket with its session cookie and `Origin`.
async fn connect(
    a: &Args,
    n: usize,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    String,
> {
    let mut req = a
        .url
        .as_str()
        .into_client_request()
        .map_err(|e| e.to_string())?;
    let name = if a.origin.starts_with("https://") {
        "__Host-vesper_session"
    } else {
        "vesper_session"
    };
    let cookie = format!("{name}={}", a.tokens[n]);
    for (k, v) in [("cookie", cookie.as_str()), ("origin", a.origin.as_str())] {
        let v = v
            .parse()
            .map_err(|_| format!("speaker {n}: bad {k} header"))?;
        req.headers_mut().insert(k, v);
    }
    let (ws, _) = tokio_tungstenite::connect_async_with_config(req, None, true)
        .await
        .map_err(|e| format!("speaker {n}: connect: {e}"))?;
    Ok(ws)
}

/// Drop speaker `s`'s TCP connection with no close frame and no `leave`, as a lost network
/// does (a clean close ends the call), then open a new socket and send `resume`. Returns the time
/// from starting the new connect to `resumed`. The fake client stays up meanwhile.
async fn reconnect(a: &Args, n: usize, s: Speaker) -> Result<f64, String> {
    s.task.abort();
    let _ = s.task.await; // its clone of the sink is gone once it has stopped
    drop(s.sink);
    let _client = s.client;
    let start = Instant::now();
    let ws = connect(a, n).await?;
    let (mut sink, mut stream) = ws.split();
    let resume = ClientMsg::Resume {
        room_id: a.room_id,
        session: s.session,
        muted: false,
        deafened: false,
    };
    sink.send(Message::text(serde_json::to_string(&resume).unwrap()))
        .await
        .map_err(|e| e.to_string())?;
    let wait = async {
        while let Some(Ok(m)) = stream.next().await {
            let Message::Text(text) = m else { continue };
            match serde_json::from_str::<ServerMsg>(&text) {
                Ok(ServerMsg::Resumed { .. }) => return Ok(start.elapsed().as_secs_f64() * 1000.0),
                Ok(ServerMsg::ResumeFailed) => return Err(format!("speaker {n}: resume_failed")),
                Ok(ServerMsg::Error { code, message }) => {
                    return Err(format!("speaker {n}: resume: {code:?}: {message}"));
                }
                _ => {}
            }
        }
        Err(format!("speaker {n}: socket closed before resumed"))
    };
    tokio::time::timeout(Duration::from_secs(10), wait)
        .await
        .map_err(|_| format!("speaker {n}: no resumed within 10 s"))?
}

/// Join like a browser: Join with the offer, apply the Joined answer, answer every later Offer.
async fn join(a: &Args, n: usize, video: bool) -> Result<Speaker, String> {
    let start = Instant::now();
    let ws = connect(a, n).await?;
    let client = Arc::new(
        if video {
            FakeClient::with_video()
        } else {
            FakeClient::new()
        }
        .map_err(|e| e.to_string())?,
    );
    let (mut sink, mut stream) = ws.split();
    let join = ClientMsg::Join {
        room_id: a.room_id,
        offer: client.offer().into(),
    };
    sink.send(Message::text(serde_json::to_string(&join).unwrap()))
        .await
        .map_err(|e| e.to_string())?;
    let (joined_tx, joined_rx) = oneshot::channel::<Result<(String, u64, Instant), String>>();
    let sink = Arc::new(tokio::sync::Mutex::new(sink));
    let task = {
        let (client, sink) = (client.clone(), sink.clone());
        tokio::spawn(async move {
            let mut joined_tx = Some(joined_tx);
            while let Some(Ok(m)) = stream.next().await {
                let Message::Text(text) = m else { continue };
                let Ok(msg) = serde_json::from_str::<ServerMsg>(&text) else {
                    continue;
                };
                match msg {
                    ServerMsg::Joined {
                        answer,
                        user_id,
                        session,
                        ..
                    } => {
                        let at = Instant::now();
                        let r = client
                            .accept_answer(&answer)
                            .map(|()| (user_id, session, at));
                        if let Some(tx) = joined_tx.take() {
                            let _ = tx.send(r);
                        }
                    }
                    ServerMsg::Offer { sdp } => {
                        let Ok(sdp) = client.accept_offer(&sdp) else {
                            continue;
                        };
                        let a = serde_json::to_string(&ClientMsg::Answer { sdp }).unwrap();
                        if sink.lock().await.send(Message::text(a)).await.is_err() {
                            return;
                        }
                    }
                    ServerMsg::Error { code, message } => {
                        if let Some(tx) = joined_tx.take() {
                            let _ = tx.send(Err(format!("{code:?}: {message}")));
                        } else {
                            // After the join, e.g. a refused ShareStart: a setup error.
                            eprintln!("vesper-loadtest: server error: {code:?}: {message}");
                            std::process::exit(2);
                        }
                    }
                    _ => {}
                }
            }
        })
    };
    match tokio::time::timeout(Duration::from_secs(10), joined_rx).await {
        Ok(Ok(Ok((user_id, session, at)))) => Ok(Speaker {
            client,
            task,
            sink,
            user_id,
            session,
            join_ms: at.duration_since(start).as_secs_f64() * 1000.0,
        }),
        Ok(Ok(Err(e))) => Err(format!("speaker {n}: {e}")),
        _ => Err(format!("speaker {n}: no Joined within 10 s")),
    }
}

async fn run(a: Args) -> Result<bool, String> {
    if let Some(pid) = a.pid
        && cpu_ticks(pid).is_none()
    {
        return Err(format!(
            "--pid {pid}: cannot read /proc/{pid}/stat (process gone, or not Linux)"
        ));
    }
    // One at a time, as people join a call. Media-ready is joined to ICE and DTLS up.
    let mut speakers = vec![];
    let mut media_ms = vec![];
    for n in 0..a.speakers {
        let s = join(&a, n, a.sharers > 0).await?;
        let joined = Instant::now();
        while !s.client.is_connected() {
            if joined.elapsed() > Duration::from_secs(10) {
                return Err(format!("speaker {n}: media not connected within 10 s"));
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        media_ms.push(joined.elapsed().as_secs_f64() * 1000.0);
        speakers.push(s);
    }
    // Media can only flow once ICE/DTLS are up and every other voice has a negotiated track.
    let ready = Instant::now() + Duration::from_secs(20);
    while !speakers.iter().all(|s| {
        s.client.is_connected() && s.client.remote_tracks_of(MediaKind::Audio) + 1 >= a.speakers
    }) {
        if Instant::now() > ready {
            return Err("speakers not connected with all tracks within 20 s".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Sharers announce, then everyone watches every other sharer; the SFU then renegotiates a
    // video track to each viewer.
    let frame_bytes = (a.share_mbps * 1_000_000 / 8 / VIDEO_FPS) as usize;
    for s in speakers.iter().take(a.sharers) {
        s.send(&ClientMsg::ShareStart {
            fps: VIDEO_FPS as u8,
            bitrate_kbps: (a.share_mbps * 1000) as u32,
        })
        .await?;
    }
    for (i, v) in speakers.iter().enumerate() {
        for sh in speakers.iter().take(a.sharers).enumerate() {
            if sh.0 != i {
                v.send(&ClientMsg::Watch {
                    user_id: sh.1.user_id.clone(),
                    on: true,
                })
                .await?;
            }
        }
    }
    let ready = Instant::now() + Duration::from_secs(20);
    while !speakers.iter().enumerate().all(|(i, s)| {
        s.client.remote_tracks_of(MediaKind::Video) >= a.sharers - usize::from(i < a.sharers)
    }) {
        if Instant::now() > ready {
            return Err("viewers did not get every video track within 20 s".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let rss_idle = a.pid.and_then(|p| status_mib(p, "VmRSS:"));
    let cpu_start = a
        .pid
        .and_then(|p| cpu_ticks(p).map(|t| (t, Instant::now())));
    let queued = a.seconds * 1000 / a.packet_ms;
    for s in &speakers {
        s.client
            .set_audio_interval(Duration::from_millis(a.packet_ms));
        s.client.send_audio(queued as usize);
    }
    for s in speakers.iter().take(a.sharers) {
        s.client
            .send_video((a.seconds * VIDEO_FPS) as usize, frame_bytes);
    }
    tokio::time::sleep(Duration::from_secs(a.seconds)).await;
    let cpu_end = a.pid.and_then(cpu_ticks);
    let rss_load = a.pid.and_then(|p| status_mib(p, "VmRSS:"));
    let rss_peak = a.pid.and_then(|p| status_mib(p, "VmHWM:"));
    // Let packets still in flight (and the last queued sends) land.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Payload bytes 1..9 are a tag unique to the sending client.
    let tag = |p: &[u8]| {
        p.get(1..9)
            .map(|b| u64::from_be_bytes(b.try_into().unwrap()))
    };
    let sent_payloads: Vec<_> = speakers.iter().map(|s| s.client.sent()).collect();
    let sent: Vec<u64> = sent_payloads.iter().map(|p| p.len() as u64).collect();
    let tags: HashMap<u64, usize> = sent_payloads
        .iter()
        .enumerate()
        .filter_map(|(i, p)| Some((tag(p.first()?)?, i)))
        .collect();
    let received: Vec<Vec<u64>> = speakers
        .iter()
        .map(|s| {
            let mut counts = vec![0; speakers.len()];
            for p in s.client.received() {
                if let Some(&j) = tag(&p).and_then(|t| tags.get(&t)) {
                    counts[j] += 1;
                }
            }
            counts
        })
        .collect();

    // Bytes 9..17 are the packet's sequence number within its sender.
    let seq = |p: &[u8]| {
        p.get(9..17)
            .map(|b| u64::from_be_bytes(b.try_into().unwrap()))
    };
    let sent_times: Vec<HashMap<u64, Instant>> = speakers
        .iter()
        .map(|s| {
            s.client
                .sent_timed()
                .into_iter()
                .filter_map(|(at, p)| Some((seq(&p)?, at)))
                .collect()
        })
        .collect();
    let arrivals: Vec<Vec<(usize, u64, Instant)>> = speakers
        .iter()
        .map(|s| {
            s.client
                .received_timed()
                .into_iter()
                .filter_map(|(at, p)| Some((*tags.get(&tag(&p)?)?, seq(&p)?, at)))
                .collect()
        })
        .collect();
    let timing = timing(&sent_times, &arrivals);

    // Video frames carry the same sender tag as that sender's audio.
    let video_got: Vec<Vec<u64>> = speakers
        .iter()
        .map(|s| {
            let mut counts = vec![0; a.sharers];
            for p in s.client.received_video() {
                if let Some(&j) = tag(&p).and_then(|t| tags.get(&t))
                    && j < a.sharers
                {
                    counts[j] += 1;
                }
            }
            counts
        })
        .collect();

    let reports = summarize(&sent, &received);
    for (i, r) in reports.iter().enumerate() {
        let per_peer: Vec<String> = (0..sent.len())
            .filter(|&j| j != i)
            .map(|j| format!("{j}:{}", received[i][j]))
            .collect();
        println!(
            "speaker {i:>2}: sent {:>5}  received {:>6}/{:<6} loss {:>5.2}%  {}  [from peer: {}]",
            sent[i],
            r.received,
            r.expected,
            r.loss_percent,
            if r.ok { "ok" } else { "FAIL" },
            per_peer.join(" ")
        );
        if let Some((j, p)) = r.bad_pair {
            println!(
                "    pair under {PASS_PERCENT}%: speaker {i} got {p:.1}% of speaker {j}'s packets"
            );
        }
        let target = queued;
        if (sent[i] as f64) < target as f64 * 0.95 {
            println!(
                "    warning: speaker {i} sent only {} of {target} queued packets",
                sent[i]
            );
        }
    }
    let mut video_worst_percent = None;
    if a.sharers > 0 {
        let queued = a.seconds * VIDEO_FPS;
        let mut worst = u64::MAX;
        for (i, row) in video_got.iter().enumerate() {
            let per: Vec<String> = row
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(j, g)| format!("{j}:{g}"))
                .collect();
            worst = row
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, &g)| g)
                .chain(Some(worst))
                .min()
                .unwrap_or(worst);
            println!(
                "viewer {i:>2}: video frames of {queued} queued [from sharer: {}]",
                per.join(" ")
            );
        }
        println!(
            "video: {} sharers at {} Mbps, worst viewer-sharer pair got {worst}/{queued} frames ({:.1}%); not part of PASS",
            a.sharers,
            a.share_mbps,
            worst as f64 * 100.0 / queued as f64
        );
        video_worst_percent = Some(worst as f64 * 100.0 / queued as f64);
    }
    let lat = &timing.latency_ms;
    let jit = &timing.pair_jitter_ms;
    let jitter_max = jit.iter().copied().fold(0.0, f64::max);
    let jitter_mean = if jit.is_empty() {
        0.0
    } else {
        jit.iter().sum::<f64>() / jit.len() as f64
    };
    println!(
        "forwarding latency: p50 {:.2} ms  p95 {:.2} ms  p99 {:.2} ms  max {:.2} ms over {} packets",
        percentile(lat, 0.50),
        percentile(lat, 0.95),
        percentile(lat, 0.99),
        percentile(lat, 1.0),
        lat.len()
    );
    println!(
        "jitter (RFC 3550, per pair): mean {jitter_mean:.2} ms  max {jitter_max:.2} ms over {} pairs",
        jit.len()
    );
    let mut cpu_percent = None;
    if let (Some((t0, at)), Some(t1)) = (cpu_start, cpu_end) {
        let secs = at.elapsed().as_secs_f64() - 2.0;
        let pct = (t1.saturating_sub(t0)) as f64 / CLK_TCK / secs * 100.0;
        cpu_percent = Some(pct);
        println!(
            "server CPU: {pct:.1}% of one core over {secs:.1} s (Linux /proc, CLK_TCK assumed 100)"
        );
    } else if a.pid.is_some() {
        println!("server CPU: unavailable (server exited during the run?)");
    }
    if let (Some(idle), Some(load), Some(peak)) = (rss_idle, rss_load, rss_peak) {
        println!(
            "server memory: RSS {idle:.1} MiB before, {load:.1} MiB under load, peak {peak:.1} MiB"
        );
    }
    let join_ms: Vec<f64> = speakers.iter().map(|s| s.join_ms).collect();
    let mut resume_ms = vec![];
    let mut resume_errors = vec![];
    let resume_supported = speakers.first().is_some_and(|s| s.session != 0);
    if !resume_supported && a.reconnects > 0 {
        println!("reconnect: skipped, the server has no resume (joined.session is 0)");
    }
    let mut kept = vec![];
    for (n, s) in speakers.into_iter().enumerate() {
        if resume_supported && n < a.reconnects {
            match reconnect(&a, n, s).await {
                Ok(ms) => resume_ms.push(ms),
                Err(e) => resume_errors.push(e),
            }
        } else {
            kept.push(s);
        }
    }
    let speakers = kept;
    println!(
        "join: joined median {:.1} ms max {:.1} ms; media connected median {:.1} ms max {:.1} ms",
        median(&join_ms),
        join_ms.iter().copied().fold(0.0, f64::max),
        median(&media_ms),
        media_ms.iter().copied().fold(0.0, f64::max),
    );
    if !resume_ms.is_empty() || !resume_errors.is_empty() {
        println!(
            "reconnect: {} resumed, median {:.1} ms; {} failed {:?}",
            resume_ms.len(),
            median(&resume_ms),
            resume_errors.len(),
            resume_errors
        );
    }
    for s in &speakers {
        s.task.abort();
    }
    let pass = reports.iter().all(|r| r.ok);
    if let Some(path) = &a.json {
        let (got, expected) = reports
            .iter()
            .fold((0, 0), |(g, e), r| (g + r.received, e + r.expected));
        let worst_pair = (0..sent.len())
            .flat_map(|i| {
                (0..sent.len())
                    .filter(move |&j| j != i)
                    .map(move |j| (i, j))
            })
            .map(|(i, j)| {
                if sent[j] == 0 {
                    0.0
                } else {
                    received[i][j].min(sent[j]) as f64 * 100.0 / sent[j] as f64
                }
            })
            .fold(100.0, f64::min);
        let summary = serde_json::json!({
            "speakers": a.speakers,
            "sharers": a.sharers,
            "share_mbps": a.share_mbps,
            "seconds": a.seconds,
            "pass_95": pass,
            "packets_sent": sent.iter().sum::<u64>(),
            "packets_expected": expected,
            "packets_received": got,
            "loss_percent": if expected == 0 { 100.0 } else { 100.0 - got as f64 * 100.0 / expected as f64 },
            "worst_pair_percent": worst_pair,
            "latency_ms": {
                "count": lat.len(),
                "p50": percentile(lat, 0.50),
                "p95": percentile(lat, 0.95),
                "p99": percentile(lat, 0.99),
                "max": percentile(lat, 1.0),
            },
            "jitter_ms": { "pairs": jit.len(), "mean": jitter_mean, "max": jitter_max },
            "server_cpu_percent": cpu_percent,
            "server_rss_mib": { "idle": rss_idle, "load": rss_load, "peak": rss_peak },
            "video_worst_pair_percent": video_worst_percent,
            "packet_ms": a.packet_ms,
            "forwarded_per_second": got as f64 / a.seconds as f64,
            "join_ms": join_ms,
            "media_ms": media_ms,
            "resume_supported": resume_supported,
            "resume_ms": resume_ms,
            "resume_errors": resume_errors,
        });
        std::fs::write(path, format!("{summary:#}\n"))
            .map_err(|e| format!("--json {path}: {e}"))?;
    }
    Ok(pass)
}

#[tokio::main]
async fn main() {
    let args = parse_args().unwrap_or_else(|e| {
        eprintln!(
            "vesper-loadtest: {e}\nusage: vesper-loadtest --url ws://127.0.0.1:PORT/ws --origin http://127.0.0.1:PORT --tokens FILE --room-id N --speakers N --seconds S [--sharers N] [--share-mbps M] [--packet-ms MS] [--reconnects N] [--pid PID] [--json FILE]"
        );
        std::process::exit(2);
    });
    match run(args).await {
        Ok(true) => println!("PASS"),
        Ok(false) => {
            println!("FAIL: a speaker received under {PASS_PERCENT}% of its peers' packets");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("vesper-loadtest: {e}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two speakers sending 100 each; fixture rows are what each one received.
    fn two(a_got: u64, b_got: u64) -> Vec<SpeakerReport> {
        summarize(&[100, 100], &[vec![0, a_got], vec![b_got, 0]])
    }

    #[test]
    fn summary_fails_below_95_percent() {
        let r = two(94, 100);
        assert!(!r[0].ok);
        assert!(r[1].ok);
        assert!((r[0].loss_percent - 6.0).abs() < 1e-9);
    }

    #[test]
    fn one_dead_peer_among_30_speakers_fails() {
        let n = 30;
        let sent = vec![100; n];
        let mut rx = vec![vec![100; n]; n];
        for row in rx.iter_mut() {
            row[7] = 0; // speaker 7 never delivers to anyone
        }
        let r = summarize(&sent, &rx);
        // Aggregate is ~96.6% for receiver 0, yet it must fail on the pair.
        assert!(r[0].loss_percent < 5.0);
        assert!(!r[0].ok);
        assert_eq!(r[0].bad_pair, Some((7, 0.0)));
    }

    #[test]
    fn one_pair_at_94_percent_fails_despite_good_aggregate() {
        let mut rx = vec![vec![100; 12]; 12];
        rx[0][1] = 94;
        let r = summarize(&[100; 12], &rx);
        assert!(r[0].loss_percent < 1.0);
        assert!(!r[0].ok);
    }

    #[test]
    fn every_pair_at_exactly_95_passes() {
        let mut rx = vec![vec![95; 12]; 12];
        assert!(summarize(&[100; 12], &rx).iter().all(|r| r.ok));
        rx[3][4] = 94;
        assert!(!summarize(&[100; 12], &rx)[3].ok);
    }

    #[test]
    fn passes_at_exactly_95_percent() {
        assert!(two(95, 95).iter().all(|r| r.ok));
    }

    #[test]
    fn speaker_who_received_nothing_fails() {
        let r = two(0, 100);
        assert!(!r[0].ok);
        assert_eq!(r[0].loss_percent, 100.0);
    }

    #[test]
    fn single_speaker_passes_trivially() {
        let r = summarize(&[250], &[vec![0]]);
        assert!(r[0].ok);
        assert_eq!(r[0].loss_percent, 0.0);
    }

    #[test]
    fn received_above_sent_is_clamped() {
        let r = two(150, 100);
        assert_eq!(r[0].received, 100);
        assert_eq!(r[0].loss_percent, 0.0);
    }

    #[test]
    fn percentile_is_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&v, 0.5), 50.0);
        assert_eq!(percentile(&v, 0.99), 99.0);
        assert_eq!(percentile(&v, 1.0), 100.0);
        assert_eq!(percentile(&v, 0.0), 1.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
    }

    #[test]
    fn median_of_odd_even_and_empty() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
        assert_eq!(median(&[]), 0.0);
    }

    #[test]
    fn steady_transit_has_no_jitter() {
        assert_eq!(rfc3550_jitter(&[3.0; 50]), 0.0);
        assert_eq!(rfc3550_jitter(&[]), 0.0);
    }

    #[test]
    fn jitter_follows_rfc3550_update() {
        // One 16 ms step: J = 16/16 = 1, then decays by 15/16 per steady packet.
        assert!((rfc3550_jitter(&[0.0, 16.0]) - 1.0).abs() < 1e-9);
        assert!((rfc3550_jitter(&[0.0, 16.0, 16.0]) - 15.0 / 16.0).abs() < 1e-9);
    }

    #[test]
    fn timing_pairs_arrivals_with_sends() {
        let t0 = Instant::now();
        let ms = |n: u64| Duration::from_millis(n);
        let sent = vec![
            HashMap::from([(0, t0), (1, t0 + ms(20))]),
            HashMap::from([(0, t0 + ms(5))]),
        ];
        // Speaker 1 got both of speaker 0's packets (2 ms and 4 ms late); speaker 0 got one of
        // speaker 1's (1 ms late) and one with a sequence number nobody sent, which is skipped.
        let arrivals = vec![
            vec![(1, 0, t0 + ms(6)), (1, 9, t0 + ms(7))],
            vec![(0, 0, t0 + ms(2)), (0, 1, t0 + ms(24))],
        ];
        let t = timing(&sent, &arrivals);
        assert_eq!(t.latency_ms, vec![1.0, 2.0, 4.0]);
        // Only the 1 <- 0 pair has two packets: |4 - 2| / 16.
        assert_eq!(t.pair_jitter_ms, vec![2.0 / 16.0]);
    }

    #[test]
    fn peers_that_sent_nothing_fail() {
        assert!(!summarize(&[0, 0], &[vec![0, 0], vec![0, 0]])[0].ok);
    }
}
