//! Watching a screen share through a real `vesper-server` and its SFU on loopback: a fake sharer sends VP8 (made by
//! the pure Rust `vp8-encoder`) and a desktop client watches it, decodes it and stops.
#![cfg(unix)] // the server crates use Unix file permissions; Windows CI runs the unit tests

mod common;

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Client, two};
use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::format::Codec;
use str0m::media::{Direction, Frequency, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event as RtcEvent, Input, Output, Rtc};
use vesper_desktop::model::{Phase, WATCH_GRACE};
use vesper_desktop::net::{Cmd, Event};
use vesper_desktop::voice::Frame;
use vesper_protocol::{ClientMsg, ServerMsg};

const W: u32 = 160;
const H: u32 = 90;
/// BT.601 limited range: red and blue.
const RED: (u8, u8, u8) = (81, 90, 240);
const BLUE: (u8, u8, u8) = (41, 240, 110);

/// What the sharer's media thread and the test share.
struct Media {
    rtc: Rtc,
    pending: Option<SdpPendingOffer>,
    video: Mid,
    connected: bool,
    colour: (u8, u8, u8),
    keyframe_requests: usize,
    /// Video m-lines the SFU offered the sharer: one would mean the desktop client sends video.
    remote_video: usize,
}

/// A browser-like sharer: a str0m peer with a send-only Opus and VP8 m-line, sending a solid colour at 30 fps,
/// with a keyframe whenever the SFU asks for one. Its socket is a desktop `Client` (for sign in and signaling).
struct Sharer {
    client: Client,
    media: Arc<Mutex<Media>>,
    offer: String,
    stop: Arc<AtomicBool>,
    watchers: u32,
}

impl Sharer {
    fn new(client: Client) -> Sharer {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = socket.local_addr().unwrap();
        let mut rtc = Rtc::builder()
            .clear_codecs()
            .enable_opus(true, false)
            .enable_vp8(true)
            .build(Instant::now());
        rtc.add_local_candidate(Candidate::host(addr, "udp").unwrap());
        let mut change = rtc.sdp_api();
        change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
        let video = change.add_media(MediaKind::Video, Direction::SendOnly, None, None, None);
        let (offer, pending) = change.apply().unwrap();
        let media = Arc::new(Mutex::new(Media {
            rtc,
            pending: Some(pending),
            video,
            connected: false,
            colour: RED,
            keyframe_requests: 0,
            remote_video: 0,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (m, s) = (media.clone(), stop.clone());
        std::thread::spawn(move || drive(socket, addr, &m, &s));
        Sharer {
            client,
            media,
            offer: offer.to_sdp_string(),
            stop,
            watchers: 0,
        }
    }

    fn signal(&self, m: ClientMsg) {
        self.client.run(vec![Cmd::Signal(m)]);
    }

    /// Its socket's events: the SFU's answer and offers go to the media, the watcher count is kept.
    fn pump(&mut self) {
        let media = self.media.clone();
        let mut watchers = None;
        self.client.pump_with(|ev| {
            let Event::Server(m) = ev else { return vec![] };
            let mut md = media.lock().unwrap();
            match &**m {
                ServerMsg::Joined { answer, .. } => {
                    let pending = md.pending.take().unwrap();
                    let answer = SdpAnswer::from_sdp_string(answer).unwrap();
                    md.rtc.sdp_api().accept_answer(pending, answer).unwrap();
                    vec![]
                }
                ServerMsg::Offer { sdp } => {
                    let offer = SdpOffer::from_sdp_string(sdp).unwrap();
                    let answer = md.rtc.sdp_api().accept_offer(offer).unwrap();
                    vec![Cmd::Signal(ClientMsg::Answer {
                        sdp: answer.to_sdp_string(),
                    })]
                }
                ServerMsg::Watchers { count } => {
                    watchers = Some(*count);
                    vec![]
                }
                _ => vec![],
            }
        });
        if let Some(n) = watchers {
            self.watchers = n;
        }
    }

    fn media(&self) -> std::sync::MutexGuard<'_, Media> {
        self.media.lock().unwrap()
    }
}

impl Drop for Sharer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn drive(socket: UdpSocket, addr: SocketAddr, media: &Mutex<Media>, stop: &AtomicBool) {
    let pic = |c: (u8, u8, u8)| {
        let n = (W.div_ceil(2) * H.div_ceil(2)) as usize;
        let mut i420 = vec![c.0; (W * H) as usize];
        i420.extend(std::iter::repeat_n(c.1, n));
        i420.extend(std::iter::repeat_n(c.2, n));
        i420
    };
    let config = vp8_encoder::EncoderConfig::new(W, H, 30, 300_000).with_keyframe_interval(None);
    let mut enc = vp8_encoder::Encoder::new(config).unwrap();
    let (mut sent, mut next) = (0u64, Instant::now());
    let mut buf = vec![0; 2000];
    while !stop.load(Ordering::Relaxed) {
        let timeout = {
            let mut m = media.lock().unwrap();
            loop {
                match m.rtc.poll_output() {
                    Ok(Output::Timeout(t)) => break t,
                    Ok(Output::Transmit(t)) => {
                        let _ = socket.send_to(&t.contents, t.destination);
                    }
                    Ok(Output::Event(RtcEvent::Connected)) => m.connected = true,
                    Ok(Output::Event(RtcEvent::KeyframeRequest(_))) => {
                        m.keyframe_requests += 1;
                        enc.request_keyframe();
                    }
                    Ok(Output::Event(RtcEvent::MediaAdded(a))) if a.kind == MediaKind::Video => {
                        m.remote_video += 1;
                    }
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
        };
        let wait = timeout
            .saturating_duration_since(Instant::now())
            .clamp(Duration::from_millis(1), Duration::from_millis(5));
        socket.set_read_timeout(Some(wait)).unwrap();
        let got = socket.recv_from(&mut buf).ok();
        let mut m = media.lock().unwrap();
        if let Some((n, source)) = got
            && let Ok(contents) = buf[..n].try_into()
        {
            let r = Receive {
                proto: Protocol::Udp,
                source,
                destination: addr,
                contents,
            };
            let _ = m.rtc.handle_input(Input::Receive(Instant::now(), r));
        }
        let now = Instant::now();
        if m.rtc.handle_input(Input::Timeout(now)).is_err() {
            return;
        }
        if !m.connected || now < next {
            continue;
        }
        next = now + Duration::from_millis(33);
        let i420 = pic(m.colour);
        let p = vp8_encoder::Picture::from_i420(&i420, W, H).unwrap();
        let Some(frame) = enc.encode(&p, false).unwrap() else {
            continue;
        };
        let video = m.video;
        let Some(w) = m.rtc.writer(video) else {
            continue;
        };
        let pt = w
            .payload_params()
            .find(|p| p.spec().codec == Codec::Vp8)
            .unwrap()
            .pt();
        let time = MediaTime::new(sent * 3000, Frequency::NINETY_KHZ);
        if w.write(pt, now, time, frame.data).is_ok() {
            sent += 1;
        }
    }
}

/// The newest picture the viewer got: whose, its size and its mean colour (RGB). The test takes each picture and
/// drops it, as the view does, so the next can come.
#[derive(Default)]
struct Seen(Option<(String, u32, u32, [u32; 3])>);

impl Seen {
    fn take(&mut self, f: Frame) {
        let n = u64::from(f.width * f.height);
        let sum = |c: usize| {
            f.bgra
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| u64::from(p[c]))
                .sum::<u64>()
        };
        let rgb = [2, 1, 0].map(|c| (sum(c) / n) as u32);
        self.0 = Some((f.user_id, f.width, f.height, rgb));
    }

    fn colour(&self) -> Option<[u32; 3]> {
        self.0.as_ref().map(|s| s.3)
    }
}

/// Pumps the viewer and the sharer until `done` holds.
async fn until(
    a: &mut Client,
    b: &mut Sharer,
    seen: &mut Seen,
    what: &str,
    done: impl Fn(&Client, &Sharer, &Seen) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        a.pump();
        b.pump();
        for (_, f) in a.model.call.take_frames() {
            seen.take(f);
        }
        if done(a, b, seen) {
            return;
        }
        if Instant::now() > deadline {
            let m = b.media();
            panic!(
                "timed out waiting for {what}: viewer pictures {}, hidden {:?}, phase {:?}; sharer watchers {}, \
                 keyframe requests {}, connected {}",
                a.model.call.pictures,
                a.model.call.hidden,
                a.model.call.phase,
                b.watchers,
                m.keyframe_requests,
                m.connected
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn reddish(c: [u32; 3]) -> bool {
    c[0] > 200 && c[1] < 60 && c[2] < 60
}

fn blueish(c: [u32; 3]) -> bool {
    c[2] > 200 && c[0] < 60 && c[1] < 60
}

#[tokio::test(flavor = "multi_thread")]
async fn a_desktop_client_watches_decodes_and_stops_watching_a_share() {
    let (_s, mut a, b) = two().await;
    let room = a.room("general");
    let bob = b.id();
    let mut b = Sharer::new(b);
    let mut seen = Seen::default();

    // Ann (desktop) is in the call first; bob joins with video, so the SFU renegotiates ann's video m-line.
    let cmds = a.model.join_voice(room);
    a.run(cmds);
    until(&mut a, &mut b, &mut seen, "ann connected", |a, _, _| {
        a.model.call.connected
    })
    .await;
    b.signal(ClientMsg::Join {
        room_id: room,
        offer: b.offer.clone(),
    });
    until(&mut a, &mut b, &mut seen, "bob connected", |_, b, _| {
        b.media().connected
    })
    .await;
    b.signal(ClientMsg::ShareStart {
        fps: 30,
        bitrate_kbps: 1000,
    });
    until(
        &mut a,
        &mut b,
        &mut seen,
        "ann sees bob share",
        |a, _, _| !a.model.sharers().is_empty(),
    )
    .await;
    assert_eq!(a.model.sharers()[0].0, bob);
    assert!(seen.0.is_none(), "the view is not on screen yet");

    // The view on screen: the share is watched with no click (it has a tile), the server's watch goes out, the SFU
    // asks bob for a keyframe, and red pictures of bob's size come.
    let now = Instant::now();
    let cmds = a.model.set_share_visible(true, now);
    a.run(cmds);
    until(&mut a, &mut b, &mut seen, "red pictures", |a, b, s| {
        a.model.call.pictures >= 5 && s.colour().is_some_and(reddish) && b.watchers == 1
    })
    .await;
    let (id, w, h, _) = seen.0.clone().unwrap();
    assert_eq!((id.as_str(), w, h), (bob.as_str(), W, H));
    println!("red came out as {:?} (RGB)", seen.colour().unwrap());
    assert!(
        b.media().keyframe_requests >= 1,
        "the watch asked for a keyframe"
    );

    // The picture follows the share.
    b.media().colour = BLUE;
    until(&mut a, &mut b, &mut seen, "blue pictures", |_, _, s| {
        s.colour().is_some_and(blueish)
    })
    .await;
    println!("blue came out as {:?} (RGB)", seen.colour().unwrap());

    // Hide it: after the grace the server's watch off goes, bob has no watcher, and no picture comes.
    let cmds = a.model.hide_share(&bob, true, Instant::now());
    a.run(cmds);
    let cmds = a.model.sync_watches(Instant::now() + WATCH_GRACE);
    a.run(cmds);
    until(&mut a, &mut b, &mut seen, "no watcher", |_, b, _| {
        b.watchers == 0
    })
    .await;
    let (pictures, then) = (a.model.call.pictures, Instant::now());
    until(&mut a, &mut b, &mut seen, "half a second", |_, _, _| {
        then.elapsed() > Duration::from_millis(500)
    })
    .await;
    assert_eq!(
        a.model.call.pictures, pictures,
        "no pictures once unwatched"
    );
    assert!(a.model.call.frames.is_empty());

    // Show it again (a fresh decoder waits for the keyframe the watch asks for), then bob stops: the view clears.
    seen.0 = None;
    let cmds = a.model.hide_share(&bob, false, Instant::now());
    a.run(cmds);
    until(&mut a, &mut b, &mut seen, "pictures again", |a, _, s| {
        a.model.call.pictures >= pictures + 2 && s.colour().is_some_and(blueish)
    })
    .await;
    b.signal(ClientMsg::ShareStop);
    until(&mut a, &mut b, &mut seen, "the view cleared", |a, _, _| {
        a.model.sharers().is_empty()
    })
    .await;
    assert!(a.model.call.frames.is_empty());
    assert_eq!(a.model.call.phase, Phase::Joined, "the call goes on");

    // The desktop client's video m-line sends, as the web's: the SFU offered bob a video m-line from it.
    assert_eq!(b.media().remote_video, 1);
}

/// Ann (desktop) shares the test pattern through the real capture, encode and send path; bob (desktop) watches it
/// through the SFU and decodes the colour bars.
#[tokio::test(flavor = "multi_thread")]
async fn a_desktop_client_shares_its_screen_with_another() {
    let (_s, mut a, mut b) = two().await;
    let room = a.room("general");
    let ann = a.id();
    let cmds = a.model.join_voice(room);
    a.run(cmds);
    let cmds = b.model.join_voice(room);
    b.run(cmds);
    common::until_all(&mut [&mut a, &mut b], "both connected", |cs| {
        cs.iter().all(|c| c.model.call.connected)
    })
    .await;

    // Start: the screen opens, share_start goes, bob sees ann share.
    let cmds = a.model.share(30, 1000, true);
    a.run(cmds);
    common::until_all(&mut [&mut a, &mut b], "bob sees ann share", |cs| {
        cs[0].model.call.mine.is_some_and(|m| m.open) && !cs[1].model.sharers().is_empty()
    })
    .await;
    assert_eq!(b.model.sharers()[0].0, ann);

    // Bob watches: ann's watcher count turns her encoder on, and the bars come out at her size.
    let now = Instant::now();
    let mut cmds = b.model.set_share_visible(true, now);
    cmds.extend(b.model.sync_watches(now));
    b.run(cmds);
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last = None;
    while b.model.call.pictures < 10 || a.model.call.mine.and_then(|m| m.figures).is_none() {
        a.pump();
        b.pump();
        // A copy: the next picture comes only once this one is dropped, as the view does.
        if let Some(f) = b.model.call.take_frames().remove(&ann) {
            last = Some((f.width, f.height, f.bgra.to_vec()));
        }
        assert!(
            Instant::now() < deadline,
            "timed out: bob has {} pictures, ann's share {:?}",
            b.model.call.pictures,
            a.model.call.mine
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (w, h, bgra) = last.expect("a picture");
    assert_eq!((w, h), (320, 180));
    assert_eq!(a.model.call.mine.unwrap().watchers, 1);
    // Each bar's middle on row 90: red, green, blue, white (BGRA). The bars slide a pixel a frame, so look near.
    let px = |x: usize| {
        let i = (90 * 320 + x) * 4;
        [bgra[i + 2], bgra[i + 1], bgra[i]]
    };
    let near = |got: [u8; 3], want: [u8; 3]| got.iter().zip(want).all(|(g, w)| g.abs_diff(w) < 60);
    let bars = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]];
    let seen: Vec<[u8; 3]> = [40, 120, 200, 280].map(px).to_vec();
    assert!(
        bars.iter().all(|b| seen.iter().any(|s| near(*s, *b))),
        "the four bars: {seen:?}"
    );
    println!("ann's share: {:?}", a.model.call.mine.unwrap().figures);

    // Stop: bob's view clears and ann's share is gone.
    let cmds = a.model.stop_share();
    a.run(cmds);
    common::until_all(&mut [&mut a, &mut b], "bob's view cleared", |cs| {
        cs[1].model.sharers().is_empty()
    })
    .await;
    assert!(a.model.call.mine.is_none());
}
