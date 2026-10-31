#![forbid(unsafe_code)]

//! Synthetic voice load: N fake browsers join one room over the real WebSocket protocol, each
//! sends 20 ms Opus-sized packets, and every speaker counts what it gets from each other speaker.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use vesper_protocol::{ClientMsg, ServerMsg};
use vesper_sfu::testing::FakeClient;

const PASS_PERCENT: f64 = 95.0;
const PACKETS_PER_SECOND: u64 = 50;

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
    key: String,
    room: String,
    speakers: usize,
    seconds: u64,
    pid: Option<u32>,
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
    Ok(Args {
        url: get("--url")?,
        key: get("--key")?,
        room: get("--room")?,
        speakers,
        seconds,
        pid: match m.get("--pid") {
            Some(p) => Some(p.parse().map_err(|e| format!("--pid: {e}"))?),
            None => None,
        },
    })
}

/// utime + stime in clock ticks from /proc/<pid>/stat (Linux only).
fn cpu_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The comm field may contain spaces; fields resume after the last ')'. utime/stime are fields 14/15.
    let rest: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    Some(rest.get(11)?.parse::<u64>().ok()? + rest.get(12)?.parse::<u64>().ok()?)
}

/// shortcut: assumes CLK_TCK = 100 (true on Linux x86/ARM); reading it needs libc/unsafe.
const CLK_TCK: f64 = 100.0;

struct Speaker {
    client: Arc<FakeClient>,
    task: JoinHandle<()>,
}

/// Join like a browser: Join with the offer, apply the Joined answer, answer every later Offer.
async fn join(a: &Args, n: usize) -> Result<Speaker, String> {
    let (ws, _) = tokio_tungstenite::connect_async(&a.url)
        .await
        .map_err(|e| format!("speaker {n}: connect: {e}"))?;
    let client = Arc::new(FakeClient::new().map_err(|e| e.to_string())?);
    let (mut sink, mut stream) = ws.split();
    let join = ClientMsg::Join {
        room: a.room.clone(),
        name: format!("load-{n}"),
        key: a.key.clone(),
        offer: client.offer().into(),
    };
    sink.send(Message::text(serde_json::to_string(&join).unwrap()))
        .await
        .map_err(|e| e.to_string())?;
    let (joined_tx, joined_rx) = oneshot::channel::<Result<(), String>>();
    let task = {
        let client = client.clone();
        tokio::spawn(async move {
            let mut joined_tx = Some(joined_tx);
            while let Some(Ok(m)) = stream.next().await {
                let Message::Text(text) = m else { continue };
                let Ok(msg) = serde_json::from_str::<ServerMsg>(&text) else {
                    continue;
                };
                match msg {
                    ServerMsg::Joined { answer, .. } => {
                        let r = client.accept_answer(&answer);
                        if let Some(tx) = joined_tx.take() {
                            let _ = tx.send(r);
                        }
                    }
                    ServerMsg::Offer { sdp } => {
                        let Ok(sdp) = client.accept_offer(&sdp) else {
                            continue;
                        };
                        let a = serde_json::to_string(&ClientMsg::Answer { sdp }).unwrap();
                        if sink.send(Message::text(a)).await.is_err() {
                            return;
                        }
                    }
                    ServerMsg::Error { code, message } => {
                        if let Some(tx) = joined_tx.take() {
                            let _ = tx.send(Err(format!("{code:?}: {message}")));
                        }
                    }
                    _ => {}
                }
            }
        })
    };
    match tokio::time::timeout(Duration::from_secs(10), joined_rx).await {
        Ok(Ok(Ok(()))) => Ok(Speaker { client, task }),
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
    let mut speakers = vec![];
    for n in 0..a.speakers {
        speakers.push(join(&a, n).await?);
    }
    // Media can only flow once ICE/DTLS are up and every other voice has a negotiated track.
    let ready = Instant::now() + Duration::from_secs(20);
    while !speakers
        .iter()
        .all(|s| s.client.is_connected() && s.client.remote_tracks() + 1 >= a.speakers)
    {
        if Instant::now() > ready {
            return Err("speakers not connected with all tracks within 20 s".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let cpu_start = a
        .pid
        .and_then(|p| cpu_ticks(p).map(|t| (t, Instant::now())));
    for s in &speakers {
        s.client
            .send_audio((a.seconds * PACKETS_PER_SECOND) as usize);
    }
    tokio::time::sleep(Duration::from_secs(a.seconds)).await;
    let cpu_end = a.pid.and_then(cpu_ticks);
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
        let target = a.seconds * PACKETS_PER_SECOND;
        if (sent[i] as f64) < target as f64 * 0.95 {
            println!(
                "    warning: speaker {i} sent only {} of {target} queued packets",
                sent[i]
            );
        }
    }
    if let (Some((t0, at)), Some(t1)) = (cpu_start, cpu_end) {
        let secs = at.elapsed().as_secs_f64() - 2.0;
        println!(
            "server CPU: {:.1}% of one core over {secs:.1} s (Linux /proc, CLK_TCK assumed 100)",
            (t1.saturating_sub(t0)) as f64 / CLK_TCK / secs * 100.0
        );
    } else if a.pid.is_some() {
        println!("server CPU: unavailable (server exited during the run?)");
    }
    for s in &speakers {
        s.task.abort();
    }
    Ok(reports.iter().all(|r| r.ok))
}

#[tokio::main]
async fn main() {
    let args = parse_args().unwrap_or_else(|e| {
        eprintln!(
            "vesper-loadtest: {e}\nusage: vesper-loadtest --url ws://127.0.0.1:PORT/ws --key K --room R --speakers N --seconds S [--pid PID]"
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
    fn peers_that_sent_nothing_fail() {
        assert!(!summarize(&[0, 0], &[vec![0, 0], vec![0, 0]])[0].ok);
    }
}
