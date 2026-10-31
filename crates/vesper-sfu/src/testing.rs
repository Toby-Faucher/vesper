//! A fake WebRTC client for tests (feature `test-support`): a str0m `Rtc` playing the browser
//! over real loopback UDP, driven by its own thread.
//!
//! Join flow: send [`FakeClient::offer`] to the SFU, pass the answer to
//! [`FakeClient::accept_answer`], and answer each SFU renegotiation offer with
//! [`FakeClient::accept_offer`]. Then [`FakeClient::send_audio`] and inspect
//! [`FakeClient::received`].

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
    remote_tracks: usize,
    to_send: usize,
    next_send: Instant,
    sent: Vec<Vec<u8>>,
    received: Vec<Vec<u8>>,
}

impl FakeClient {
    /// Bind a loopback UDP socket and prepare an offer for one send-only Opus track.
    pub fn new() -> std::io::Result<Self> {
        static TAG: AtomicU64 = AtomicU64::new(1);
        let socket = UdpSocket::bind("127.0.0.1:0")?;
        let addr = socket.local_addr()?;
        let mut rtc = Rtc::builder()
            .clear_codecs()
            .enable_opus(true, false)
            .build(Instant::now());
        let candidate = Candidate::host(addr, "udp").map_err(std::io::Error::other)?;
        rtc.add_local_candidate(candidate);
        let mut change = rtc.sdp_api();
        let send_mid = change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
        let (offer, pending) = change.apply().expect("adding media is a change");

        let shared = Arc::new(Mutex::new(State {
            rtc,
            tag: TAG.fetch_add(1, Ordering::Relaxed),
            send_mid,
            pending: Some(pending),
            connected: false,
            remote_tracks: 0,
            to_send: 0,
            next_send: Instant::now(),
            sent: vec![],
            received: vec![],
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

    /// The initial SDP offer (one send-only Opus track, loopback host candidate).
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

    /// Payloads sent so far, in order.
    pub fn sent(&self) -> Vec<Vec<u8>> {
        self.state().sent.clone()
    }

    /// Media payloads received so far (from any track), in arrival order.
    pub fn received(&self) -> Vec<Vec<u8>> {
        self.state().received.clone()
    }

    /// Whether ICE and DTLS are up, so media can flow.
    pub fn is_connected(&self) -> bool {
        self.state().connected
    }

    /// Number of incoming audio tracks negotiated by the SFU (other users' voices).
    pub fn remote_tracks(&self) -> usize {
        self.state().remote_tracks
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
                        self.remote_tracks += 1;
                    }
                    Event::MediaData(d) => self.received.push(d.data.to_vec()),
                    _ => {}
                },
                Err(_) => return Instant::now() + Duration::from_millis(5),
            }
        }
    }

    fn maybe_send(&mut self) {
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
