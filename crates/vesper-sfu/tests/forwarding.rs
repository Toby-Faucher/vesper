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
        ..Default::default()
    })
    .expect("sfu spawns")
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

/// One test's SFU plus a sink for errors raised in background answer tasks, so a failure there
/// shows up as itself instead of as a later timeout.
struct Ctx {
    h: SfuHandle,
    errors: Arc<Mutex<Vec<String>>>,
}

impl Ctx {
    fn new(cfg: SfuConfig) -> Ctx {
        Ctx {
            h: Sfu::spawn(cfg).expect("sfu spawns"),
            errors: Arc::default(),
        }
    }

    /// An SFU reachable on loopback: its own socket address is the only candidate.
    fn loopback() -> Ctx {
        let t = Ctx::new(SfuConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        });
        t.h.set_advertised(vec![t.h.local_addr()]);
        t
    }

    /// Join like the server will: send the offer, apply the answer, then answer every SFU offer.
    async fn join(&self, room: &str, user: &str) -> Peer {
        self.join_with(room, user, true).await
    }

    /// Like `join`, but never answers SFU offers (they are still logged).
    async fn join_silent(&self, room: &str, user: &str) -> Peer {
        self.join_with(room, user, false).await
    }

    async fn join_with(&self, room: &str, user: &str, answer: bool) -> Peer {
        let client = Arc::new(FakeClient::new().expect("fake client"));
        let ok = self
            .h
            .join(room.into(), user.into(), client.offer().to_string())
            .await
            .expect("join");
        client
            .accept_answer(&ok.answer_sdp)
            .expect("answer accepted");

        let log = Arc::new(Mutex::new(Vec::new()));
        let (c, l, h, user) = (
            client.clone(),
            log.clone(),
            self.h.clone(),
            user.to_string(),
        );
        let errors = self.errors.clone();
        let mut events = ok.events;
        tokio::spawn(async move {
            while let Some(ev) = events.recv().await {
                match ev {
                    SfuEvent::Offer(sdp) => {
                        if answer {
                            match c.accept_offer(&sdp) {
                                Ok(a) => h.answer(&user, a),
                                Err(e) => {
                                    errors
                                        .lock()
                                        .unwrap()
                                        .push(format!("{user} rejected sfu offer: {e}\n{sdp}"));
                                    return;
                                }
                            }
                        }
                        l.lock().unwrap().push(Logged::Offer(sdp));
                    }
                    SfuEvent::Closed => l.lock().unwrap().push(Logged::Closed),
                }
            }
        });
        Peer { client, log }
    }

    async fn wait_until(&self, what: &str, timeout: Duration, mut cond: impl FnMut() -> bool) {
        let deadline = Instant::now() + timeout;
        while !cond() {
            if let Some(e) = self.errors.lock().unwrap().first() {
                panic!("while waiting for {what}: {e}");
            }
            assert!(Instant::now() < deadline, "timed out waiting for: {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Wait until `listener` is connected and has negotiated `tracks` incoming tracks.
    async fn ready(&self, listener: &Peer, tracks: usize) {
        self.wait_until("connected", CONNECT, || listener.client.is_connected())
            .await;
        self.wait_until("remote tracks", CONNECT, || {
            listener.client.remote_tracks() == tracks
        })
        .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn forwards_audio_between_two_clients() {
    let t = Ctx::loopback();
    let a = t.join("r", "a").await;
    let b = t.join("r", "b").await;
    t.wait_until("a connected", CONNECT, || a.client.is_connected())
        .await;
    t.ready(&b, 1).await;

    a.client.send_audio(50);
    t.wait_until("a sent 50", CONNECT, || a.client.sent().len() == 50)
        .await;
    let sent = a.client.sent();
    t.wait_until("b received 45", CONNECT, || b.client.received().len() >= 45)
        .await;

    for p in b.client.received() {
        assert!(sent.contains(&p), "received bytes not sent verbatim: {p:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sender_does_not_hear_itself() {
    let t = Ctx::loopback();
    let a = t.join("r", "a").await;
    let b = t.join("r", "b").await;
    t.ready(&a, 1).await;
    t.ready(&b, 1).await;

    a.client.send_audio(50);
    t.wait_until("b received 45", CONNECT, || b.client.received().len() >= 45)
        .await;

    // A's only incoming track is B's (silent); its own track never comes back.
    assert_eq!(a.client.remote_tracks(), 1);
    assert!(a.client.received().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn rooms_are_isolated() {
    let t = Ctx::loopback();
    // x joins between y1 and y2: covers both the join-time and the propagated subscription path.
    let y1 = t.join("y", "y1").await;
    let x = t.join("x", "x1").await;
    let y2 = t.join("y", "y2").await;
    t.wait_until("x connected", CONNECT, || x.client.is_connected())
        .await;
    t.wait_until("y1 connected", CONNECT, || y1.client.is_connected())
        .await;
    t.ready(&y2, 1).await;

    y1.client.send_audio(50);
    t.wait_until("y2 received 45", CONNECT, || {
        y2.client.received().len() >= 45
    })
    .await;

    assert!(x.offers().is_empty(), "x was offered a track from room y");
    assert_eq!(x.client.remote_tracks(), 0);
    assert!(x.client.received().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn abrupt_disconnect_closes_client() {
    let t = Ctx::loopback();
    let a = t.join("r", "a").await;
    let b = t.join("r", "b").await;
    t.ready(&a, 1).await;
    t.ready(&b, 1).await;
    let offers_before = b.offers().len();

    // Vanish without a goodbye: no more UDP from A at all.
    a.client.stop();

    t.wait_until("a closed", ICE_TIMEOUT, || a.closed()).await;
    t.wait_until("b offered a's track stopped", CONNECT, || {
        b.offers()[offers_before..]
            .iter()
            .any(|sdp| sdp.contains("m=audio 0 "))
    })
    .await;
    assert!(!b.closed());
}

#[tokio::test(flavor = "multi_thread")]
async fn forwarded_track_msid_is_sender_user_id() {
    let t = Ctx::loopback();
    let _alice = t.join("r", "alice-42").await;
    let bob = t.join("r", "bob-7").await;
    t.ready(&bob, 1).await;

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
    // The latest offer lists every m-line: alice's stream must be in exactly one.
    let latest = offers.last().unwrap();
    let alice_lines = latest
        .lines()
        .filter(|l| l.starts_with("a=msid:alice-42 "))
        .count();
    assert_eq!(alice_lines, 1, "duplicate m-lines for alice:\n{latest}");
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
    let t = Ctx::loopback();
    let err =
        t.h.join("r".into(), "u".into(), "not sdp".into())
            .await
            .unwrap_err();
    assert!(matches!(err, vesper_sfu::JoinError::BadOffer(_)), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn unanswered_offer_closes_client() {
    let t = Ctx::new(SfuConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        answer_timeout: Duration::from_millis(500),
        ..Default::default()
    });
    t.h.set_advertised(vec![t.h.local_addr()]);
    let a = t.join_silent("r", "a").await;
    let b = t.join("r", "b").await;

    // B's arrival makes the SFU offer A a track; A never answers.
    t.wait_until("a offered b's track", CONNECT, || !a.offers().is_empty())
        .await;
    t.wait_until("a closed", CONNECT, || a.closed()).await;
    assert!(!b.closed());
}

#[tokio::test(flavor = "multi_thread")]
async fn leave_after_rejoin_closes_the_live_session() {
    let t = Ctx::loopback();
    let first = t.join("r", "u").await;

    // Re-join and leave back to back, so both usually land in one command drain while the
    // superseded session is still in the SFU's list.
    let c = FakeClient::new().unwrap();
    let (rejoin, ()) = tokio::join!(
        t.h.join("r".into(), "u".into(), c.offer().to_string()),
        async { t.h.leave("u") }
    );
    let mut events = rejoin.expect("rejoin").events;

    t.wait_until("first closed", CONNECT, || first.closed())
        .await;
    let ev = tokio::time::timeout(CONNECT, events.recv()).await;
    assert_eq!(
        ev,
        Ok(Some(SfuEvent::Closed)),
        "leave did not end the live session"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hostile_offers_are_rejected() {
    use str0m::Rtc;
    use str0m::media::{Direction, MediaKind};

    let t = Ctx::loopback();
    let offer = |media: &[MediaKind]| {
        let mut rtc = Rtc::builder().build(Instant::now());
        let mut change = rtc.sdp_api();
        for kind in media {
            change.add_media(*kind, Direction::RecvOnly, None, None, None);
        }
        change.apply().unwrap().0.to_sdp_string()
    };

    let many_audio = offer(&[MediaKind::Audio; 50]);
    let with_video = offer(&[MediaKind::Audio, MediaKind::Video]);
    for (name, sdp) in [
        ("50 audio m-lines", many_audio),
        ("audio + video", with_video),
    ] {
        let err = t.h.join("r".into(), "evil".into(), sdp).await.unwrap_err();
        assert!(
            matches!(err, vesper_sfu::JoinError::BadOffer(_)),
            "{name}: {err:?}"
        );
    }
}

/// Media flows when the socket is bound to a wildcard address, so packets arrive "on" an
/// address that is not itself a candidate (the Docker / NAT case).
async fn forwards_with_bind(bind: &str) {
    let t = Ctx::new(SfuConfig {
        bind: bind.parse().unwrap(),
        ..Default::default()
    });
    t.h.set_advertised(vec![SocketAddr::new(
        "127.0.0.1".parse().unwrap(),
        t.h.local_addr().port(),
    )]);
    let a = t.join("r", "a").await;
    let b = t.join("r", "b").await;
    t.wait_until("a connected", CONNECT, || a.client.is_connected())
        .await;
    t.ready(&b, 1).await;
    a.client.send_audio(20);
    t.wait_until("b received 15", CONNECT, || b.client.received().len() >= 15)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wildcard_ipv4_bind_uses_advertised_address() {
    forwards_with_bind("0.0.0.0:0").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dual_stack_bind_serves_ipv4_clients() {
    if std::net::UdpSocket::bind("[::]:0").is_err() {
        eprintln!("skipping: no IPv6 on this host");
        return;
    }
    forwards_with_bind("[::]:0").await;
}
