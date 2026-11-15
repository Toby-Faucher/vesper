//! One call on its own thread: a str0m client (like the SFU's `FakeClient`), its UDP socket, Opus, and the devices.
//!
//! Every 20 ms the engine takes one frame from each person's jitter buffer, decodes it (or conceals a lost one), and
//! mixes them into the output; that mix is also the echo canceller's far end. Then it cleans what the microphone gave
//! and sends it, encoded, on its one audio m-line.
//!
//! A device that is lost mid-call is opened again (the saved one, else the default) every `REOPEN`, without ending
//! the call, and the call bar says so meanwhile.
//!
//! Screen shares come in on the video m-lines the SFU adds. The engine passes a watched share's VP8 frames to the
//! decoder thread (`video.rs`) and asks the sharer for a keyframe (a PLI, which the SFU passes on) when it needs one.

use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use opus::{Application, Bitrate, Channels, Decoder, Encoder};
use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::format::Codec;
use str0m::media::{Direction, Frequency, KeyframeRequestKind, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};

use super::denoise::{Capture, Late, Loaded, deep_filter};
use super::echo::{self, Echo};
use super::jitter::{Jitter, Pop};
use super::mix::{Limiter, Speaking, mix_into, rms, volume_to_gain};
use super::video::Video;
use super::{AudioDevices, AudioInput, AudioOutput, FRAME, RATE, Setup, VoiceCmd, VoiceEvent};

const TICK: Duration = Duration::from_millis(20);
/// Captured audio held at most, so a stalled engine does not send old sound later (100 ms).
const MAX_CAPTURED: usize = FRAME * 5;
/// Media must be up this long after the answer.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// One keyframe request per share in this time at most: a keyframe of a big screen costs every viewer.
const KEYFRAME_EVERY: Duration = Duration::from_secs(1);
/// A share counts as struggling this long after lost packets or a frame that did not decode (the web's 6 s window).
const STRUGGLE_FOR: Duration = Duration::from_secs(6);
/// Packet loss above this, in a receiver report, is struggling (the web's 2 %).
const STRUGGLE_LOSS: f32 = 0.02;
/// How often a lost device is tried again.
const REOPEN: Duration = Duration::from_secs(1);

pub const NO_CONNECT: &str = "Could not reach the voice server. A firewall may block UDP.";
pub const LOST: &str = "The voice connection was lost.";
pub const MIC_LOST: &str = "Your microphone was disconnected.";
pub const SPEAKERS_LOST: &str = "Your speakers were disconnected.";
pub const BOTH_LOST: &str = "Your microphone and speakers were disconnected.";

struct Peer {
    jitter: Jitter,
    decoder: Decoder,
    speaking: Speaking,
}

struct Engine<'a> {
    rtc: Rtc,
    socket: UdpSocket,
    /// Each SFU address and our address towards it (the socket is bound to 0.0.0.0).
    routes: Vec<(SocketAddr, SocketAddr)>,
    mid: Mid,
    pending: Option<SdpPendingOffer>,
    answered: Option<Instant>,
    connected: bool,
    muted: bool,
    deafened: bool,
    devices: &'a dyn AudioDevices,
    /// The saved device names.
    names: (Option<String>, Option<String>),
    /// `None` while lost.
    input: Option<Box<dyn AudioInput>>,
    output: Option<Box<dyn AudioOutput>>,
    /// When to try a lost device again, and what the call bar was told about the devices.
    reopen: Instant,
    devices_shown: &'static str,
    encoder: Encoder,
    /// The echo canceller, then the noise filter, between the microphone and Opus.
    echo: Echo,
    capture: Capture,
    captured: Vec<f32>,
    /// Frames captured so far, sent or not: the RTP clock keeps running while muted.
    frames: u64,
    peers: HashMap<String, Peer>,
    /// Percent by user id.
    volumes: HashMap<String, f32>,
    me: Speaking,
    limiter: Limiter,
    shown: (bool, Vec<String>),
    video: Video,
    /// Shares being decoded, by user id.
    watching: HashSet<String>,
    /// Each sharer's video m-line, by user id.
    video_mids: HashMap<String, Mid>,
    /// Shares whose last frame was dropped: the next one needs a keyframe.
    gaps: HashSet<String>,
    /// When each share was last asked for a keyframe.
    asked: HashMap<String, Instant>,
    /// When each share last lost packets or frames, and the shares shown as struggling.
    trouble: HashMap<String, Instant>,
    struggling: HashSet<String>,
    /// Packets received per audio m-line at the last stats window, and this window's samples by user.
    packets: HashMap<Mid, u64>,
    links: Vec<(String, crate::link::Sample)>,
    emit: &'a dyn Fn(VoiceEvent),
}

pub fn run<'a>(
    devices: &'a dyn AudioDevices,
    setup: Setup,
    denoiser: Receiver<Loaded>,
    video: Video,
    cmds: Receiver<VoiceCmd>,
    emit: &'a dyn Fn(VoiceEvent),
) {
    let mut e = match Engine::new(devices, setup, denoiser, video, emit) {
        Ok(e) => e,
        Err(text) => return emit(VoiceEvent::Failed(text)),
    };
    if let Err(text) = e.run(cmds) {
        emit(VoiceEvent::Failed(text.into()));
    }
}

impl<'a> Engine<'a> {
    fn new(
        devices: &'a dyn AudioDevices,
        setup: Setup,
        denoiser: Receiver<Loaded>,
        video: Video,
        emit: &'a dyn Fn(VoiceEvent),
    ) -> Result<Self, String> {
        let input = devices.input(setup.input.as_deref())?;
        let output = devices.output(setup.output.as_deref())?;
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .map_err(|e| format!("Could not open a network socket for the call: {e}"))?;
        let mut encoder = Encoder::new(RATE, Channels::Mono, Application::Voip)
            .map_err(|e| format!("Could not start the Opus encoder: {e}"))?;
        let bits = setup.bitrate_kbps.clamp(32, 510) as i32 * 1000;
        encoder
            .set_bitrate(Bitrate::Bits(bits))
            .map_err(|e| format!("Could not set the bitrate: {e}"))?;
        let mut rtc = Rtc::builder()
            .clear_codecs()
            .enable_opus(true, false)
            .enable_vp8(true)
            .set_stats_interval(Some(Duration::from_secs(2)))
            .build(Instant::now());
        // One audio m-line both ways, as the browser's addTrack makes. Others' voices come on m-lines the SFU adds.
        // One receive-only video m-line: an offer with a video m-line gets others' screens (ADR 0009), and as this
        // one never sends, the SFU makes no track of it and offers nobody video from this client. (The web's
        // m-line is send-only, for its own share.)
        let mut change = rtc.sdp_api();
        let mid = change.add_media(MediaKind::Audio, Direction::SendRecv, None, None, None);
        change.add_media(MediaKind::Video, Direction::RecvOnly, None, None, None);
        let (offer, pending) = change.apply().expect("adding media is a change");
        emit(VoiceEvent::Offer(offer.to_sdp_string()));
        Ok(Engine {
            rtc,
            socket,
            routes: vec![],
            mid,
            pending: Some(pending),
            answered: None,
            connected: false,
            muted: false,
            deafened: false,
            devices,
            names: (setup.input, setup.output),
            input: Some(input),
            output: Some(output),
            reopen: Instant::now(),
            devices_shown: "",
            encoder,
            echo: Echo::new(setup.echo, echo::late()),
            capture: Capture::new(setup.denoise, denoiser, deep_filter, Late::default()),
            captured: vec![],
            frames: 0,
            peers: HashMap::new(),
            volumes: setup.volumes,
            me: Speaking::default(),
            limiter: Limiter::default(),
            shown: (false, vec![]),
            video,
            watching: HashSet::new(),
            video_mids: HashMap::new(),
            gaps: HashSet::new(),
            asked: HashMap::new(),
            trouble: HashMap::new(),
            struggling: HashSet::new(),
            packets: HashMap::new(),
            links: vec![],
            emit,
        })
    }

    fn run(&mut self, cmds: Receiver<VoiceCmd>) -> Result<(), &'static str> {
        let mut buf = vec![0; 2000];
        let mut tick = Instant::now() + TICK;
        loop {
            loop {
                match cmds.try_recv() {
                    Ok(VoiceCmd::Stop) | Err(TryRecvError::Disconnected) => return Ok(()),
                    Ok(c) => self.command(c)?,
                    Err(TryRecvError::Empty) => break,
                }
            }
            let timeout = self.poll()?;
            let wait = timeout
                .min(tick)
                .saturating_duration_since(Instant::now())
                .clamp(Duration::from_millis(1), TICK);
            self.socket.set_read_timeout(Some(wait)).map_err(|_| LOST)?;
            match self.socket.recv_from(&mut buf) {
                Ok((n, source)) => self.receive(&buf[..n], source),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                // A port unreachable after a send comes back as an error on Linux; ICE copes.
                Err(_) => {}
            }
            while let Ok(need) = self.video.needs.try_recv() {
                self.need_keyframe(&need.user_id, need.broken);
            }
            let now = Instant::now();
            self.rtc
                .handle_input(Input::Timeout(now))
                .map_err(|_| LOST)?;
            if now >= tick {
                self.check_devices();
                self.tick();
                self.struggle();
                tick += TICK;
                if tick < now {
                    tick = now + TICK; // fell behind: skip, do not burst
                }
            }
            if !self.connected && self.answered.is_some_and(|t| t.elapsed() > CONNECT_TIMEOUT) {
                return Err(NO_CONNECT);
            }
        }
    }

    fn command(&mut self, cmd: VoiceCmd) -> Result<(), &'static str> {
        match cmd {
            VoiceCmd::Answer(sdp) => {
                let pending = self.pending.take().ok_or(LOST)?;
                let answer = SdpAnswer::from_sdp_string(&sdp).map_err(|_| LOST)?;
                self.rtc
                    .sdp_api()
                    .accept_answer(pending, answer)
                    .map_err(|_| LOST)?;
                self.add_routes(&sdp)?;
                self.answered = Some(Instant::now());
            }
            VoiceCmd::Offer(sdp) => {
                let offer = SdpOffer::from_sdp_string(&sdp).map_err(|_| LOST)?;
                let answer = self.rtc.sdp_api().accept_offer(offer).map_err(|_| LOST)?;
                (self.emit)(VoiceEvent::Answer(answer.to_sdp_string()));
            }
            VoiceCmd::Muted(m) => self.muted = m,
            VoiceCmd::Deafened(d) => self.deafened = d,
            VoiceCmd::Volume { user_id, percent } => {
                self.volumes.insert(user_id, percent);
            }
            VoiceCmd::Watch { user_id, on } => {
                if on {
                    self.watching.insert(user_id.clone());
                } else {
                    self.watching.remove(&user_id);
                    self.trouble.remove(&user_id);
                }
                self.video.watch(user_id, on);
            }
            VoiceCmd::Start { .. } | VoiceCmd::Stop => {}
        }
        Ok(())
    }

    /// A host candidate for our side towards each SFU address in the answer, found by asking the OS which local
    /// address it would use. The SFU learns our public address from the ICE checks themselves.
    fn add_routes(&mut self, answer: &str) -> Result<(), &'static str> {
        let port = self.socket.local_addr().map_err(|_| LOST)?.port();
        for remote in sfu_addresses(answer)
            .into_iter()
            .filter(SocketAddr::is_ipv4)
        {
            let Ok(probe) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else {
                continue;
            };
            let Ok(ip) = probe
                .connect(remote)
                .and_then(|()| probe.local_addr())
                .map(|a| a.ip())
            else {
                continue;
            };
            let local = SocketAddr::new(ip, port);
            if let Ok(c) = Candidate::host(local, "udp") {
                self.rtc.add_local_candidate(c);
                self.routes.push((remote, local));
            }
        }
        if self.routes.is_empty() {
            return Err(NO_CONNECT);
        }
        Ok(())
    }

    fn receive(&mut self, data: &[u8], source: SocketAddr) {
        let Some(&(_, destination)) = self
            .routes
            .iter()
            .find(|(r, _)| *r == source)
            .or(self.routes.first())
        else {
            return;
        };
        let Ok(contents) = data.try_into() else {
            return;
        };
        let input = Input::Receive(
            Instant::now(),
            Receive {
                proto: Protocol::Udp,
                source,
                destination,
                contents,
            },
        );
        // A stray packet is not an error worth ending the call for.
        let _ = self.rtc.handle_input(input);
    }

    /// Sends what str0m wants sent and handles its events, until its next timeout.
    fn poll(&mut self) -> Result<Instant, &'static str> {
        loop {
            match self.rtc.poll_output().map_err(|_| LOST)? {
                Output::Timeout(t) => return Ok(t),
                Output::Transmit(t) => {
                    let _ = self.socket.send_to(&t.contents, t.destination);
                }
                Output::Event(Event::Connected) => {
                    self.connected = true;
                    (self.emit)(VoiceEvent::Connected);
                }
                Output::Event(Event::IceConnectionStateChange(
                    IceConnectionState::Disconnected,
                )) => {
                    return Err(LOST);
                }
                Output::Event(Event::MediaData(d)) => {
                    // The SFU labels each forwarded track with the sender's user_id as its MSID stream id.
                    let Some((kind, user)) = self
                        .rtc
                        .media(d.mid)
                        .map(|m| (m.kind(), m.stream_id().to_string()))
                        .filter(|(_, s)| !s.is_empty())
                    else {
                        continue;
                    };
                    if kind == MediaKind::Video {
                        self.screen(user, d.mid, d.contiguous, d.data.to_vec());
                        continue;
                    }
                    let Some(peer) = self.peer(&user) else {
                        continue;
                    };
                    let rtp = d.time.rebase(Frequency::FORTY_EIGHT_KHZ).numer();
                    peer.jitter.push(rtp, d.data.to_vec());
                }
                // Every 2 s: the call's own round trip and loss, and each person's audio, for the quality icons.
                Output::Event(Event::PeerStats(s)) => {
                    let loss = [s.egress_loss_fraction, s.ingress_loss_fraction]
                        .into_iter()
                        .flatten()
                        .reduce(f32::max);
                    let own = crate::link::Sample {
                        loss_pct: loss.map(|l| l * 100.),
                        jitter_ms: None,
                        rtt_ms: s.rtt.map(|d| d.as_secs_f32() * 1000.),
                        packets: None,
                    };
                    let links = std::mem::take(&mut self.links);
                    (self.emit)(VoiceEvent::Stats { own, links });
                }
                Output::Event(Event::MediaIngressStats(s))
                    if self
                        .rtc
                        .media(s.mid)
                        .is_some_and(|m| m.kind() == MediaKind::Audio) =>
                {
                    let Some(user) = self
                        .rtc
                        .media(s.mid)
                        .map(|m| m.stream_id().to_string())
                        .filter(|u| !u.is_empty())
                    else {
                        continue;
                    };
                    let got = s
                        .packets
                        .saturating_sub(self.packets.insert(s.mid, s.packets).unwrap_or(s.packets));
                    self.links.push((
                        user,
                        crate::link::Sample {
                            // shortcut: str0m's loss is the last receiver report's fraction, not this window's
                            // count; it is shown only over enough packets, as the web does.
                            loss_pct: s
                                .loss
                                .filter(|_| got >= crate::link::MIN_PACKETS)
                                .map(|l| l * 100.),
                            // RTP timestamp units of the 48 kHz Opus clock.
                            jitter_ms: Some(s.jitter as f32 / 48.),
                            rtt_ms: None,
                            packets: Some(got),
                        },
                    ));
                }
                Output::Event(Event::MediaIngressStats(s))
                    if s.loss.is_some_and(|l| l > STRUGGLE_LOSS) =>
                {
                    if let Some(user) = self
                        .video_mids
                        .iter()
                        .find(|(_, m)| **m == s.mid)
                        .map(|(u, _)| u.clone())
                    {
                        self.trouble.insert(user, Instant::now());
                    }
                }
                Output::Event(_) => {}
            }
        }
    }

    /// A VP8 frame of `user`'s screen: to the decoder while watched. A frame dropped because the decoder is behind
    /// breaks the ones after it, so the next goes with `gap` and a keyframe is asked for.
    fn screen(&mut self, user: String, mid: Mid, contiguous: bool, data: Vec<u8>) {
        if !self.watching.contains(&user) {
            return;
        }
        self.video_mids.insert(user.clone(), mid);
        let gap = self.gaps.remove(&user) || !contiguous;
        if !self.video.frame(user.clone(), data, gap) {
            self.gaps.insert(user.clone());
            self.need_keyframe(&user, true);
        }
    }

    /// Asks `user` for a keyframe through the SFU, at most once a `KEYFRAME_EVERY`. `broken` counts as struggling.
    fn need_keyframe(&mut self, user: &str, broken: bool) {
        if broken {
            self.trouble.insert(user.into(), Instant::now());
        }
        if self
            .asked
            .get(user)
            .is_some_and(|t| t.elapsed() < KEYFRAME_EVERY)
        {
            return;
        }
        let Some(&mid) = self.video_mids.get(user) else {
            return;
        };
        if let Some(mut w) = self.rtc.writer(mid)
            && w.request_keyframe(None, KeyframeRequestKind::Pli).is_ok()
        {
            self.asked.insert(user.into(), Instant::now());
        }
    }

    /// Tells the model which watched shares struggle, when that changes.
    fn struggle(&mut self) {
        let now: HashSet<String> = self
            .trouble
            .iter()
            .filter(|(u, t)| self.watching.contains(*u) && t.elapsed() < STRUGGLE_FOR)
            .map(|(u, _)| u.clone())
            .collect();
        for user in now.symmetric_difference(&self.struggling) {
            (self.emit)(VoiceEvent::Struggling {
                user_id: user.clone(),
                on: now.contains(user),
            });
        }
        self.struggling = now;
    }

    fn peer(&mut self, user: &str) -> Option<&mut Peer> {
        if !self.peers.contains_key(user) {
            let decoder = Decoder::new(RATE, Channels::Mono).ok()?;
            let peer = Peer {
                jitter: Jitter::default(),
                decoder,
                speaking: Speaking::default(),
            };
            self.peers.insert(user.to_string(), peer);
        }
        self.peers.get_mut(user)
    }

    /// Notices a lost device, and opens it again (the saved one, else the default) every `REOPEN` until it is back.
    fn check_devices(&mut self) {
        if self.input.as_ref().is_some_and(|i| !i.alive()) {
            (self.input, self.reopen) = (None, Instant::now() + REOPEN);
        }
        if self.output.as_ref().is_some_and(|o| !o.alive()) {
            (self.output, self.reopen) = (None, Instant::now() + REOPEN);
        }
        if (self.input.is_none() || self.output.is_none()) && Instant::now() >= self.reopen {
            let (d, (mic, speakers)) = (self.devices, &self.names);
            if self.input.is_none() {
                self.input = d.input(mic.as_deref()).or_else(|_| d.input(None)).ok();
            }
            if self.output.is_none() {
                self.output = d
                    .output(speakers.as_deref())
                    .or_else(|_| d.output(None))
                    .ok();
            }
            self.reopen = Instant::now() + REOPEN;
        }
        let text = match (self.input.is_none(), self.output.is_none()) {
            (true, true) => BOTH_LOST,
            (true, false) => MIC_LOST,
            (false, true) => SPEAKERS_LOST,
            (false, false) => "",
        };
        if text != self.devices_shown {
            self.devices_shown = text;
            (self.emit)(VoiceEvent::Devices(text.into()));
        }
    }

    /// 20 ms: play one frame of everyone, send what was captured, tell the model who speaks.
    fn tick(&mut self) {
        let mut mix = [0f32; FRAME];
        let mut pcm = [0f32; FRAME];
        let mut speaking = vec![];
        for (id, p) in &mut self.peers {
            let n = match p.jitter.pop() {
                Pop::Frame(data) => p.decoder.decode_float(&data, &mut pcm, false),
                Pop::Lost => p.decoder.decode_float(&[], &mut pcm, false),
                Pop::Silent => Ok(0),
            }
            .unwrap_or(0);
            pcm[n..].fill(0.0);
            if p.speaking.update(rms(&pcm)) {
                speaking.push(id.clone());
            }
            let volume = self.volumes.get(id).copied().unwrap_or(100.0);
            mix_into(&mut mix, &pcm, volume_to_gain(volume));
        }
        self.limiter.process(&mut mix);
        if self.deafened || self.output.is_none() {
            mix.fill(0.0);
        }
        if let Some(o) = &mut self.output {
            o.write(&mix);
        }
        // The far end is exactly what went to the speakers: nothing while deafened or without them.
        self.echo.render(&mix);

        if let Some(i) = &mut self.input {
            i.read(&mut self.captured);
        }
        if self.captured.len() > MAX_CAPTURED {
            let extra = self.captured.len() - MAX_CAPTURED;
            self.captured.drain(..extra);
        }
        let mut me = self.shown.0 && !self.muted;
        while self.captured.len() >= FRAME {
            let mut frame: Vec<f32> = self.captured.drain(..FRAME).collect();
            // Mic, echo cancellation, the noise filter, then Opus. The canceller runs while muted too, so it stays in
            // step with the far end; the filter skips the work, as nothing is sent.
            // shortcut: no input gain yet (the web has one).
            if let Some(why) = self.echo.capture(&mut frame) {
                (self.emit)(VoiceEvent::EchoOff(why.into()));
            }
            if let Some(status) = self.capture.process(&mut frame, !self.muted) {
                (self.emit)(VoiceEvent::Filter(status));
            }
            let level = if self.muted { 0.0 } else { rms(&frame) };
            me = self.me.update(level);
            self.send(&frame);
        }

        speaking.sort();
        if self.shown != (me, speaking.clone()) {
            self.shown = (me, speaking.clone());
            (self.emit)(VoiceEvent::Speaking {
                me,
                peers: speaking,
            });
        }
    }

    fn send(&mut self, frame: &[f32]) {
        let time = MediaTime::new(self.frames * FRAME as u64, Frequency::FORTY_EIGHT_KHZ);
        self.frames += 1;
        if self.muted || !self.connected {
            return;
        }
        let mut out = [0u8; 1500];
        let Ok(n) = self.encoder.encode_float(frame, &mut out) else {
            return;
        };
        let Some(writer) = self.rtc.writer(self.mid) else {
            return;
        };
        let Some(pt) = writer
            .payload_params()
            .find(|p| p.spec().codec == Codec::Opus)
            .map(|p| p.pt())
        else {
            return;
        };
        let _ = writer.write(pt, Instant::now(), time, out[..n].to_vec());
    }
}

/// The UDP addresses in an SDP's `a=candidate` lines.
pub fn sfu_addresses(sdp: &str) -> Vec<SocketAddr> {
    sdp.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.trim().strip_prefix("a=candidate:")?.split(' ').collect();
            if !f.get(2)?.eq_ignore_ascii_case("udp") {
                return None;
            }
            Some(SocketAddr::new(
                f.get(4)?.parse().ok()?,
                f.get(5)?.parse().ok()?,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::settings::Denoise;

    /// Devices that can be unplugged (`mic` and `speakers` false) and refuse to open (`refuse`). Each open is logged.
    #[derive(Default)]
    struct Fake {
        played: Arc<Mutex<Vec<f32>>>,
        mic: Arc<AtomicBool>,
        speakers: Arc<AtomicBool>,
        refuse: AtomicBool,
        opened: Mutex<Vec<String>>,
    }

    struct Mic(Arc<AtomicBool>);
    impl AudioInput for Mic {
        fn read(&mut self, out: &mut Vec<f32>) {
            out.extend((0..FRAME).map(|i| 0.1 * (i as f32 * 0.05).sin()));
        }
        fn alive(&self) -> bool {
            self.0.load(Ordering::Relaxed)
        }
    }

    struct Speakers(Arc<Mutex<Vec<f32>>>, Arc<AtomicBool>);
    impl AudioOutput for Speakers {
        fn write(&mut self, samples: &[f32]) {
            self.0.lock().unwrap().extend_from_slice(samples);
        }
        fn alive(&self) -> bool {
            self.1.load(Ordering::Relaxed)
        }
    }

    impl Fake {
        fn open(&self, what: &str, name: Option<&str>) -> Result<(), String> {
            let name = name.unwrap_or("default");
            self.opened.lock().unwrap().push(format!("{what} {name}"));
            if self.refuse.load(Ordering::Relaxed) {
                return Err("No microphone was found.".into());
            }
            Ok(())
        }
    }

    impl AudioDevices for Fake {
        fn input(&self, name: Option<&str>) -> Result<Box<dyn AudioInput>, String> {
            self.open("mic", name)?;
            self.mic.store(true, Ordering::Relaxed);
            Ok(Box::new(Mic(self.mic.clone())))
        }
        fn output(&self, name: Option<&str>) -> Result<Box<dyn AudioOutput>, String> {
            self.open("speakers", name)?;
            self.speakers.store(true, Ordering::Relaxed);
            Ok(Box::new(Speakers(
                self.played.clone(),
                self.speakers.clone(),
            )))
        }
    }

    fn engine<'a>(fake: &'a Fake, emit: &'a dyn Fn(VoiceEvent)) -> Engine<'a> {
        let setup = Setup {
            bitrate_kbps: 64,
            denoise: Denoise::Off,
            echo: true,
            input: Some("USB mic".into()),
            output: Some("Headset".into()),
            ..Setup::default()
        };
        let (_, denoiser) = std::sync::mpsc::channel();
        let e = Engine::new(fake, setup, denoiser, Video::start(|_| {}), emit);
        e.unwrap_or_else(|_| panic!("the engine starts"))
    }

    /// What goes to the speakers is, sample for sample, what the echo canceller gets as its far end: someone's
    /// voice at their volume after the limiter, and silence while deafened.
    #[test]
    fn the_far_end_is_exactly_what_was_played() {
        let fake = Fake::default();
        let emit = |_| {};
        let mut e = engine(&fake, &emit);
        let mut enc = Encoder::new(RATE, Channels::Mono, Application::Voip).unwrap();
        let mut packet = [0u8; 1500];
        for f in 0..8u64 {
            let tone: Vec<f32> = (0..FRAME)
                .map(|i| 0.9 * ((f as usize * FRAME + i) as f32 * 0.06).sin())
                .collect();
            let n = enc.encode_float(&tone, &mut packet).unwrap();
            e.peer("b")
                .unwrap()
                .jitter
                .push(f * FRAME as u64, packet[..n].to_vec());
        }
        e.volumes.insert("b".into(), 300.0); // loud enough for the limiter to work
        for _ in 0..6 {
            e.tick();
        }
        let played = fake.played.lock().unwrap().clone();
        assert_eq!(played.len(), 6 * FRAME);
        assert_eq!(e.echo.far, played);
        assert!(rms(&played[4 * FRAME..]) > 0.2, "a voice is playing");
        assert!(
            played
                .iter()
                .all(|s| s.abs() <= super::super::mix::Limiter::CEILING + 1e-6)
        );

        e.deafened = true;
        e.tick();
        let played = fake.played.lock().unwrap().clone();
        assert_eq!(e.echo.far, played);
        assert!(played[6 * FRAME..].iter().all(|s| *s == 0.0), "deafened");
    }

    /// An unplugged microphone or speaker set says so, is opened again (the saved one, then the default) without
    /// ending the call, and the notice clears once both are back.
    #[test]
    fn a_lost_device_says_so_and_is_opened_again() {
        let fake = Fake::default();
        let events = RefCell::new(vec![]);
        let emit = |ev| events.borrow_mut().push(ev);
        let mut e = engine(&fake, &emit);
        let devices = |events: &RefCell<Vec<VoiceEvent>>| -> Vec<String> {
            events
                .borrow_mut()
                .drain(..)
                .filter_map(|ev| match ev {
                    VoiceEvent::Devices(t) => Some(t),
                    VoiceEvent::Failed(t) => panic!("the call ended: {t}"),
                    _ => None,
                })
                .collect()
        };
        fake.opened.lock().unwrap().clear();
        e.check_devices();
        assert!(devices(&events).is_empty(), "nothing lost");

        fake.mic.store(false, Ordering::Relaxed);
        fake.refuse.store(true, Ordering::Relaxed);
        e.check_devices();
        assert_eq!(devices(&events), [MIC_LOST]);
        assert!(e.input.is_none());
        assert!(
            fake.opened.lock().unwrap().is_empty(),
            "not at once: after REOPEN"
        );
        e.tick(); // the call goes on without a mic
        assert_eq!(fake.played.lock().unwrap().len(), FRAME);

        e.reopen = Instant::now();
        e.check_devices();
        assert_eq!(*fake.opened.lock().unwrap(), ["mic USB mic", "mic default"]);
        assert!(devices(&events).is_empty(), "still lost: no new notice");
        assert!(e.reopen > Instant::now(), "tried again later");

        fake.speakers.store(false, Ordering::Relaxed);
        e.check_devices();
        assert_eq!(devices(&events), [BOTH_LOST]);

        fake.refuse.store(false, Ordering::Relaxed);
        fake.opened.lock().unwrap().clear();
        e.reopen = Instant::now();
        e.check_devices();
        assert_eq!(
            *fake.opened.lock().unwrap(),
            ["mic USB mic", "speakers Headset"]
        );
        assert_eq!(devices(&events), [""], "cleared");

        fake.speakers.store(false, Ordering::Relaxed);
        e.check_devices();
        assert_eq!(devices(&events), [SPEAKERS_LOST]);
    }

    #[test]
    fn sfu_addresses_from_candidates() {
        let sdp = "v=0\r\na=candidate:1 1 udp 2130706175 127.0.0.1 40000 typ host\r\n\
                   a=candidate:2 1 tcp 1 10.0.0.1 9 typ host\r\na=candidate:3 1 UDP 1 ::1 40001 typ host\r\n";
        assert_eq!(
            sfu_addresses(sdp),
            [
                "127.0.0.1:40000".parse().unwrap(),
                "[::1]:40001".parse().unwrap()
            ]
        );
    }
}
