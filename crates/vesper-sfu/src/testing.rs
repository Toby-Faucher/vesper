//! A fake WebRTC client for tests (feature `test-support`): a str0m `Rtc` playing the browser
//! over real loopback UDP, driven by its own thread.
//!
//! Join flow: send [`FakeClient::offer`] to the SFU, pass the answer to
//! [`FakeClient::accept_answer`], and answer each SFU renegotiation offer with
//! [`FakeClient::accept_offer`]. Then [`FakeClient::send_audio`] and inspect
//! [`FakeClient::received`]. [`FakeClient::with_video`] adds a VP8 track
//! ([`FakeClient::send_video`], [`FakeClient::received_video`]).

use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::format::Codec;
use str0m::media::{Direction, Frequency, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};

/// Opus at 48 kHz, 20 ms frames.
const FRAME: Duration = Duration::from_millis(20);
const FRAME_TICKS: u64 = 960;
/// 64 kbps * 20 ms.
const PAYLOAD_LEN: usize = 160;
/// Video frames go at the same 20 ms pacing, on the 90 kHz video clock.
const VIDEO_FRAME_TICKS: u64 = 1800;

pub struct FakeClient {
    shared: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
    offer: String,
}

struct State {
    rtc: Rtc,
    tag: u64,
    send_mid: Mid,
    pending: Option<SdpPendingOffer>,
    connected: bool,
    /// Every incoming m-line negotiated so far, in order (a recycled mid appears again).
    remote_tracks: Vec<(Mid, MediaKind)>,
    to_send: usize,
    next_send: Instant,
    sent: Vec<Vec<u8>>,
    received: Vec<Vec<u8>>,
    received_video: Vec<Vec<u8>>,
    video: Option<VideoOut>,
}

/// The send side of [`FakeClient::with_video`].
struct VideoOut {
    mid: Mid,
    to_send: usize,
    bytes: usize,
    next_send: Instant,
    sent: u64,
}

impl FakeClient {
    /// Bind a loopback UDP socket and prepare an offer for one send-only Opus track.
    pub fn new() -> std::io::Result<Self> {
        Self::build(false)
    }

    /// Like [`new`](Self::new), plus a send-only VP8 video track.
    pub fn with_video() -> std::io::Result<Self> {
        Self::build(true)
    }

    fn build(video: bool) -> std::io::Result<Self> {
        static TAG: AtomicU64 = AtomicU64::new(1);
        let socket = UdpSocket::bind("127.0.0.1:0")?;
        let addr = socket.local_addr()?;
        let mut rtc = Rtc::builder()
            .clear_codecs()
            .enable_opus(true, false)
            .enable_vp8(video)
            .build(Instant::now());
        let candidate = Candidate::host(addr, "udp").map_err(std::io::Error::other)?;
        rtc.add_local_candidate(candidate);
        let mut change = rtc.sdp_api();
        let send_mid = change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
        let video = video.then(|| VideoOut {
            mid: change.add_media(MediaKind::Video, Direction::SendOnly, None, None, None),
            to_send: 0,
            bytes: 0,
            next_send: Instant::now(),
            sent: 0,
        });
        let (offer, pending) = change.apply().expect("adding media is a change");

        let shared = Arc::new(Mutex::new(State {
            rtc,
            tag: TAG.fetch_add(1, Ordering::Relaxed),
            send_mid,
            pending: Some(pending),
            connected: false,
            remote_tracks: vec![],
            to_send: 0,
            next_send: Instant::now(),
            sent: vec![],
            received: vec![],
            received_video: vec![],
            video,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (shared, stop) = (shared.clone(), stop.clone());
            std::thread::spawn(move || drive(socket, addr, shared, stop))
        };
        Ok(FakeClient {
            shared,
            stop,
            thread: Mutex::new(Some(thread)),
            offer: offer.to_sdp_string(),
        })
    }

    /// The initial SDP offer (one send-only Opus track, a VP8 one with video, loopback host candidate).
    pub fn offer(&self) -> &str {
        &self.offer
    }

    /// Apply the SFU's answer to [`offer`](Self::offer).
    pub fn accept_answer(&self, sdp: &str) -> Result<(), String> {
        let answer = SdpAnswer::from_sdp_string(sdp).map_err(|e| e.to_string())?;
        let mut s = self.state();
        let pending = s.pending.take().ok_or("no offer pending")?;
        s.rtc
            .sdp_api()
            .accept_answer(pending, answer)
            .map_err(|e| e.to_string())
    }

    /// Accept an SFU renegotiation offer and return the SDP answer.
    pub fn accept_offer(&self, sdp: &str) -> Result<String, String> {
        let offer = SdpOffer::from_sdp_string(sdp).map_err(|e| e.to_string())?;
        let mut s = self.state();
        let answer = s
            .rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(|e| e.to_string())?;
        Ok(answer.to_sdp_string())
    }

    /// Queue `n` Opus-shaped packets, sent at 20 ms pacing once ICE is connected.
    /// Every payload is unique to this client and packet.
    pub fn send_audio(&self, n: usize) {
        self.state().to_send += n;
    }

    /// Queue `frames` video frames of `bytes` bytes, sent at 20 ms pacing once ICE is connected.
    /// Every frame is unique to this client and frame. Panics without [`with_video`](Self::with_video).
    pub fn send_video(&self, frames: usize, bytes: usize) {
        let mut s = self.state();
        let video = s.video.as_mut().expect("client built with_video");
        video.to_send += frames;
        video.bytes = bytes;
    }

    /// Audio payloads sent so far, in order.
    pub fn sent(&self) -> Vec<Vec<u8>> {
        self.state().sent.clone()
    }

    /// Audio payloads received so far (from any audio track), in arrival order.
    pub fn received(&self) -> Vec<Vec<u8>> {
        self.state().received.clone()
    }

    /// Video frames received so far (from any video track), in arrival order.
    pub fn received_video(&self) -> Vec<Vec<u8>> {
        self.state().received_video.clone()
    }

    /// Whether ICE and DTLS are up, so media can flow.
    pub fn is_connected(&self) -> bool {
        self.state().connected
    }

    /// Number of incoming tracks of any kind negotiated by the SFU (other users' voices and screens).
    pub fn remote_tracks(&self) -> usize {
        self.state().remote_tracks.len()
    }

    /// Number of incoming tracks of `kind` negotiated by the SFU.
    pub fn remote_tracks_of(&self, kind: MediaKind) -> usize {
        let s = self.state();
        s.remote_tracks.iter().filter(|(_, k)| *k == kind).count()
    }

    /// Vanish abruptly: stop the driver thread, so no more packets (not even STUN) are sent.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.lock().unwrap().take() {
            let _ = t.join();
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.shared.lock().unwrap()
    }
}

impl Drop for FakeClient {
    fn drop(&mut self) {
        self.stop();
    }
}

fn drive(socket: UdpSocket, addr: SocketAddr, shared: Arc<Mutex<State>>, stop: Arc<AtomicBool>) {
    let mut buf = vec![0; 2000];
    while !stop.load(Ordering::Relaxed) {
        let timeout = shared.lock().unwrap().poll(&socket);
        // Short cap so SDP changes made through the API take effect promptly.
        let wait = timeout
            .saturating_duration_since(Instant::now())
            .clamp(Duration::from_millis(1), Duration::from_millis(5));
        socket
            .set_read_timeout(Some(wait))
            .expect("set read timeout");
        let received = match socket.recv_from(&mut buf) {
            Ok((n, source)) => Some((n, source)),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => None,
            Err(_) => None,
        };

        let mut s = shared.lock().unwrap();
        if let Some((n, source)) = received
            && let Ok(contents) = buf[..n].try_into()
        {
            let input = Input::Receive(
                Instant::now(),
                Receive {
                    proto: Protocol::Udp,
                    source,
                    destination: addr,
                    contents,
                },
            );
            if s.rtc.handle_input(input).is_err() {
                return;
            }
        }
        if s.rtc.handle_input(Input::Timeout(Instant::now())).is_err() {
            return;
        }
        s.maybe_send();
    }
}

impl State {
    /// Drain output until the next timeout.
    fn poll(&mut self, socket: &UdpSocket) -> Instant {
        loop {
            match self.rtc.poll_output() {
                Ok(Output::Timeout(t)) => return t,
                Ok(Output::Transmit(t)) => {
                    let _ = socket.send_to(&t.contents, t.destination);
                }
                Ok(Output::Event(e)) => match e {
                    // ICE + DTLS done; media written before this is silently dropped.
                    Event::Connected => self.connected = true,
                    Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                        self.connected = false;
                    }
                    Event::MediaAdded(m) if m.direction == Direction::RecvOnly => {
                        self.remote_tracks.push((m.mid, m.kind));
                    }
                    Event::MediaData(d) => {
                        // Latest entry for the mid: a stopped m-line can be recycled.
                        let track = self.remote_tracks.iter().rev().find(|(m, _)| *m == d.mid);
                        match track {
                            Some((_, MediaKind::Video)) => {
                                self.received_video.push(d.data.to_vec())
                            }
                            _ => self.received.push(d.data.to_vec()),
                        }
                    }
                    _ => {}
                },
                Err(_) => return Instant::now() + Duration::from_millis(5),
            }
        }
    }

    fn maybe_send(&mut self) {
        self.maybe_send_audio();
        self.maybe_send_video();
    }

    fn maybe_send_video(&mut self) {
        let now = Instant::now();
        let Some(video) = self.video.as_mut() else {
            return;
        };
        if !self.connected || video.to_send == 0 || now < video.next_send {
            return;
        }
        let Some(writer) = self.rtc.writer(video.mid) else {
            return;
        };
        let Some(pt) = writer
            .payload_params()
            .find(|p| p.spec().codec == Codec::Vp8)
            .map(|p| p.pt())
        else {
            return;
        };
        let seq = video.sent;
        // Same tag layout as audio: a fixed first byte, then a tag unique to client + frame.
        let mut payload = vec![0xF8];
        payload.extend_from_slice(&self.tag.to_be_bytes());
        payload.extend_from_slice(&seq.to_be_bytes());
        payload.resize(video.bytes.max(payload.len()), (seq % 251) as u8);
        let time = MediaTime::new(seq * VIDEO_FRAME_TICKS, Frequency::NINETY_KHZ);
        if writer.write(pt, now, time, payload).is_ok() {
            video.sent += 1;
            video.to_send -= 1;
            video.next_send = now + FRAME;
        }
    }

    fn maybe_send_audio(&mut self) {
        let now = Instant::now();
        if !self.connected || self.to_send == 0 || now < self.next_send {
            return;
        }
        let Some(writer) = self.rtc.writer(self.send_mid) else {
            return;
        };
        let Some(pt) = writer
            .payload_params()
            .find(|p| p.spec().codec == Codec::Opus)
            .map(|p| p.pt())
        else {
            return;
        };
        let seq = self.sent.len() as u64;
        // TOC 0xF8: CELT fullband 20 ms, mono, one frame. Then a tag unique to client + packet.
        let mut payload = vec![0xF8];
        payload.extend_from_slice(&self.tag.to_be_bytes());
        payload.extend_from_slice(&seq.to_be_bytes());
        payload.resize(PAYLOAD_LEN, (seq % 251) as u8);
        let time = MediaTime::new(seq * FRAME_TICKS, Frequency::FORTY_EIGHT_KHZ);
        if writer.write(pt, now, time, payload.clone()).is_ok() {
            self.sent.push(payload);
            self.to_send -= 1;
            self.next_send = now + FRAME;
        }
    }
}
