//! End-to-end SFU tests: real str0m clients (`vesper_sfu::testing::FakeClient`) over loopback UDP.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use vesper_sfu::testing::FakeClient;
use vesper_sfu::{Sfu, SfuConfig, SfuEvent, SfuHandle};

/// Generous: CI machines can be slow and ICE/DTLS takes a few round trips.
const CONNECT: Duration = Duration::from_secs(10);
/// str0m declares a silent peer gone after ~19 s by default; leave plenty of headroom.
const ICE_TIMEOUT: Duration = Duration::from_secs(45);

fn spawn_sfu(advertised: Vec<SocketAddr>) -> SfuHandle {
    Sfu::spawn(SfuConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        advertised,
    })
    .expect("sfu spawns")
}

/// An SFU reachable on loopback: its own socket address is the only candidate.
fn loopback_sfu() -> SfuHandle {
    let h = spawn_sfu(vec![]);
    h.set_advertised(vec![h.local_addr()]);
    h
}

#[derive(Debug, Clone)]
enum Logged {
    Offer(String),
    Closed,
}

struct Peer {
    client: Arc<FakeClient>,
    log: Arc<Mutex<Vec<Logged>>>,
}

impl Peer {
    fn closed(&self) -> bool {
        self.log
            .lock()
            .unwrap()
            .iter()
            .any(|l| matches!(l, Logged::Closed))
    }

    fn offers(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter_map(|l| match l {
                Logged::Offer(s) => Some(s.clone()),
                Logged::Closed => None,
            })
            .collect()
    }
}

/// Join like the server will: send the offer, apply the answer, then answer every SFU offer.
async fn join(h: &SfuHandle, room: &str, user: &str) -> Peer {
    let client = Arc::new(FakeClient::new().expect("fake client"));
    let ok = h
        .join(room.into(), user.into(), client.offer().to_string())
        .await
        .expect("join");
    client
        .accept_answer(&ok.answer_sdp)
        .expect("answer accepted");

    let log = Arc::new(Mutex::new(Vec::new()));
    let (c, l, h, user) = (client.clone(), log.clone(), h.clone(), user.to_string());
    let mut events = ok.events;
    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            match ev {
                SfuEvent::Offer(sdp) => {
                    let answer = c.accept_offer(&sdp).expect("sfu offer accepted");
                    h.answer(&user, answer);
                    l.lock().unwrap().push(Logged::Offer(sdp));
                }
                SfuEvent::Closed => l.lock().unwrap().push(Logged::Closed),
            }
        }
    });
    Peer { client, log }
}

async fn wait_until(what: &str, timeout: Duration, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Wait until `listener` is connected and has negotiated `tracks` incoming tracks.
async fn ready(listener: &Peer, tracks: usize) {
    wait_until("connected", CONNECT, || listener.client.is_connected()).await;
    wait_until("remote tracks", CONNECT, || {
        listener.client.remote_tracks() == tracks
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn forwards_audio_between_two_clients() {
    let h = loopback_sfu();
    let a = join(&h, "r", "a").await;
    let b = join(&h, "r", "b").await;
    wait_until("a connected", CONNECT, || a.client.is_connected()).await;
    ready(&b, 1).await;

    a.client.send_audio(50);
    wait_until("a sent 50", CONNECT, || a.client.sent().len() == 50).await;
    let sent = a.client.sent();
    wait_until("b received 45", CONNECT, || b.client.received().len() >= 45).await;

    for p in b.client.received() {
        assert!(sent.contains(&p), "received bytes not sent verbatim: {p:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sender_does_not_hear_itself() {
    let h = loopback_sfu();
    let a = join(&h, "r", "a").await;
    let b = join(&h, "r", "b").await;
    ready(&a, 1).await;
    ready(&b, 1).await;

    a.client.send_audio(50);
    wait_until("b received 45", CONNECT, || b.client.received().len() >= 45).await;

    // A's only incoming track is B's (silent); its own track never comes back.
    assert_eq!(a.client.remote_tracks(), 1);
    assert!(a.client.received().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn rooms_are_isolated() {
    let h = loopback_sfu();
    // x joins between y1 and y2: covers both the join-time and the propagated subscription path.
    let y1 = join(&h, "y", "y1").await;
    let x = join(&h, "x", "x1").await;
    let y2 = join(&h, "y", "y2").await;
    wait_until("x connected", CONNECT, || x.client.is_connected()).await;
    wait_until("y1 connected", CONNECT, || y1.client.is_connected()).await;
    ready(&y2, 1).await;

    y1.client.send_audio(50);
    wait_until("y2 received 45", CONNECT, || {
        y2.client.received().len() >= 45
    })
    .await;

    assert!(x.offers().is_empty(), "x was offered a track from room y");
    assert_eq!(x.client.remote_tracks(), 0);
    assert!(x.client.received().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn abrupt_disconnect_closes_client() {
    let h = loopback_sfu();
    let a = join(&h, "r", "a").await;
    let b = join(&h, "r", "b").await;
    ready(&a, 1).await;
    ready(&b, 1).await;
    let offers_before = b.offers().len();

    // Vanish without a goodbye: no more UDP from A at all.
    a.client.stop();

    wait_until("a closed", ICE_TIMEOUT, || a.closed()).await;
    wait_until("b offered a's track stopped", CONNECT, || {
        b.offers()[offers_before..]
            .iter()
            .any(|sdp| sdp.contains("m=audio 0 "))
    })
    .await;
    assert!(!b.closed());
}

#[tokio::test(flavor = "multi_thread")]
async fn forwarded_track_msid_is_sender_user_id() {
    let h = loopback_sfu();
    let _alice = join(&h, "r", "alice-42").await;
    let bob = join(&h, "r", "bob-7").await;
    ready(&bob, 1).await;

    let offers = bob.offers();
    let msid_streams: Vec<&str> = offers
        .iter()
        .flat_map(|sdp| sdp.lines())
        .filter_map(|l| l.strip_prefix("a=msid:"))
        .filter_map(|rest| rest.split_whitespace().next())
        .collect();
    assert!(
        msid_streams.contains(&"alice-42"),
        "no a=msid with alice's user id in bob's offers: {offers:#?}"
    );
    assert!(
        !msid_streams.contains(&"bob-7"),
        "bob was offered his own stream"
    );
}

fn has_candidate(sdp: &str, addr: SocketAddr) -> bool {
    sdp.lines().any(|l| {
        l.starts_with("a=candidate:")
            && l.contains(&format!(" {} {} typ host", addr.ip(), addr.port()))
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn advertised_addresses_appear_as_candidates() {
    let first: Vec<SocketAddr> = vec![
        "192.0.2.10:40000".parse().unwrap(),
        "[2001:db8::10]:40000".parse().unwrap(),
    ];
    let h = spawn_sfu(first.clone());

    let c1 = FakeClient::new().unwrap();
    let ok = h
        .join("r".into(), "u1".into(), c1.offer().to_string())
        .await
        .unwrap();
    for addr in &first {
        assert!(
            has_candidate(&ok.answer_sdp, *addr),
            "{addr} missing:\n{}",
            ok.answer_sdp
        );
    }

    let second: Vec<SocketAddr> = vec!["198.51.100.20:40001".parse().unwrap()];
    h.set_advertised(second.clone());
    let c2 = FakeClient::new().unwrap();
    let ok = h
        .join("r".into(), "u2".into(), c2.offer().to_string())
        .await
        .unwrap();
    assert!(
        has_candidate(&ok.answer_sdp, second[0]),
        "{}",
        ok.answer_sdp
    );
    for addr in &first {
        assert!(
            !has_candidate(&ok.answer_sdp, *addr),
            "stale {addr} still advertised"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_offer_is_rejected() {
    let h = loopback_sfu();
    let err = h
        .join("r".into(), "u".into(), "not sdp".into())
        .await
        .unwrap_err();
    assert!(matches!(err, vesper_sfu::JoinError::BadOffer(_)), "{err:?}");
}
