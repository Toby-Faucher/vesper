//! One SFU session: a str0m `Rtc` plus the track bookkeeping for forwarding.
//! Adapted from str0m's `examples/chat.rs`; renegotiation goes over `SfuEvent`.

use std::collections::HashSet;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use str0m::change::{SdpAnswer, SdpPendingOffer};
use str0m::media::{Direction, MediaData, MediaKind, Mid};
use str0m::{Event, IceConnectionState, Input, Output, Rtc};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, warn};

use crate::SfuEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClientId(u64);

#[derive(Debug)]
pub(crate) struct TrackIn {
    pub origin: ClientId,
    /// The sender's user id: forwarded tracks carry it as their MSID stream id, which browsers
    /// expose as `MediaStream.id` in `ontrack`.
    pub user: String,
    pub mid: Mid,
    pub kind: MediaKind,
}

#[derive(Debug)]
struct TrackOut {
    track_in: Weak<TrackIn>,
    state: TrackOutState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackOutState {
    ToOpen,
    Negotiating(Mid),
    Open(Mid),
    ToStop(Mid),
    NegotiatingStop(Mid),
}

impl TrackOut {
    fn mid(&self) -> Option<Mid> {
        match self.state {
            TrackOutState::ToOpen => None,
            TrackOutState::Negotiating(m)
            | TrackOutState::Open(m)
            | TrackOutState::ToStop(m)
            | TrackOutState::NegotiatingStop(m) => Some(m),
        }
    }
}

/// What a client's output means for the other clients.
// Short-lived queue entries; boxing MediaData per packet would only add an allocation.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Propagated {
    Noop,
    Timeout(std::time::Instant),
    TrackOpen(ClientId, Weak<TrackIn>),
    MediaData(ClientId, MediaData),
}

pub(crate) struct Client {
    pub id: ClientId,
    pub user: String,
    pub room: String,
    pub rtc: Rtc,
    /// The client's offer had a video m-line, so it can be offered others' video.
    pub video: bool,
    /// The server says this client is sharing: its video is forwarded to its watchers.
    pub sharing: bool,
    /// User ids of the sharers whose video this client gets.
    pub watching: HashSet<String>,
    events: UnboundedSender<SfuEvent>,
    /// Candidates this session was created with (see `local_addr_for`).
    advertised: Vec<SocketAddr>,
    /// Our outstanding offer and when it was sent.
    pending: Option<(SdpPendingOffer, Instant)>,
    answer_timeout: Duration,
    /// The shared socket is IPv6 (dual-stack), so IPv4 peers need v4-mapped destinations.
    socket_v6: bool,
    /// The client owns the strong refs to its incoming tracks; others hold weak ones.
    pub tracks_in: Vec<Arc<TrackIn>>,
    tracks_out: Vec<TrackOut>,
}

impl Client {
    pub fn new(
        rtc: Rtc,
        room: String,
        user: String,
        events: UnboundedSender<SfuEvent>,
        advertised: Vec<SocketAddr>,
        socket_v6: bool,
        answer_timeout: Duration,
    ) -> Client {
        static ID_COUNTER: AtomicU64 = AtomicU64::new(0);
        Client {
            id: ClientId(ID_COUNTER.fetch_add(1, Ordering::Relaxed)),
            user,
            room,
            rtc,
            video: false,
            sharing: false,
            watching: HashSet::new(),
            events,
            advertised,
            pending: None,
            answer_timeout,
            socket_v6,
            tracks_in: vec![],
            tracks_out: vec![],
        }
    }

    /// The local address to report for a datagram from `source`. str0m drops STUN that does
    /// not arrive on one of its host candidates, and with `bind` = 0.0.0.0 (or behind NAT) the
    /// socket's own address is not one, so we attribute traffic to an advertised address.
    pub fn local_addr_for(&self, source: SocketAddr, socket_addr: SocketAddr) -> SocketAddr {
        self.advertised
            .iter()
            .copied()
            .find(|a| a.is_ipv4() == source.ip().to_canonical().is_ipv4())
            .unwrap_or(socket_addr)
    }

    /// End the session and emit `SfuEvent::Closed`. Called exactly once, when the client is removed.
    pub fn close(&mut self) {
        self.rtc.disconnect();
        let _ = self.events.send(SfuEvent::Closed);
    }

    pub fn handle_input(&mut self, input: Input) {
        if !self.rtc.is_alive() {
            return;
        }
        if let Err(e) = self.rtc.handle_input(input) {
            warn!("client {} disconnected: {e:?}", self.user);
            self.rtc.disconnect();
        }
    }

    pub fn poll_output(&mut self, socket: &UdpSocket) -> Propagated {
        if !self.rtc.is_alive() {
            return Propagated::Noop;
        }
        if let Some((_, sent)) = &self.pending
            && sent.elapsed() > self.answer_timeout
        {
            // No more offers can go out until this one is answered: end it so the server rejoins.
            warn!("client {} did not answer an offer in time", self.user);
            self.rtc.disconnect();
            return Propagated::Noop;
        }
        if self.negotiate_if_needed() {
            return Propagated::Noop;
        }
        match self.rtc.poll_output() {
            Ok(output) => self.handle_output(output, socket),
            Err(e) => {
                warn!("client {} poll_output failed: {e:?}", self.user);
                self.rtc.disconnect();
                Propagated::Noop
            }
        }
    }

    fn handle_output(&mut self, output: Output, socket: &UdpSocket) -> Propagated {
        match output {
            Output::Transmit(t) => {
                let destination = match t.destination {
                    SocketAddr::V4(v4) if self.socket_v6 => {
                        SocketAddr::new(v4.ip().to_ipv6_mapped().into(), v4.port())
                    }
                    d => d,
                };
                if let Err(e) = socket.send_to(&t.contents, destination) {
                    // UDP send errors (e.g. ICMP unreachable) are transient; ICE handles loss.
                    debug!("send_to {} failed: {e}", t.destination);
                }
                Propagated::Noop
            }
            Output::Timeout(t) => Propagated::Timeout(t),
            Output::Event(e) => match e {
                Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                    // As in the spike: no ICE restart, a lost connection ends the session.
                    self.rtc.disconnect();
                    Propagated::Noop
                }
                // The offer check allows one audio and one video m-line, so at most two of these.
                Event::MediaAdded(m)
                    if matches!(m.direction, Direction::RecvOnly | Direction::SendRecv) =>
                {
                    let track = Arc::new(TrackIn {
                        origin: self.id,
                        user: self.user.clone(),
                        mid: m.mid,
                        kind: m.kind,
                    });
                    let weak = Arc::downgrade(&track);
                    self.tracks_in.push(track);
                    Propagated::TrackOpen(self.id, weak)
                }
                Event::MediaData(data) => Propagated::MediaData(self.id, data),
                _ => Propagated::Noop,
            },
        }
    }

    fn negotiate_if_needed(&mut self) -> bool {
        if self.pending.is_some() {
            return false;
        }

        // An Open track whose origin went away needs its m-line stopped.
        for track in &mut self.tracks_out {
            if let TrackOutState::Open(m) = track.state
                && track.track_in.upgrade().is_none()
            {
                track.state = TrackOutState::ToStop(m);
            }
        }
        // A track whose origin left before it was ever negotiated needs no m-line at all.
        self.tracks_out
            .retain(|t| t.state != TrackOutState::ToOpen || t.track_in.strong_count() > 0);

        let mut change = self.rtc.sdp_api();
        for track in &mut self.tracks_out {
            match track.state {
                TrackOutState::ToOpen => {
                    if let Some(track_in) = track.track_in.upgrade() {
                        // str0m drops non-token chars (RFC 8830) and truncates to 64; user ids
                        // must stay within that to round-trip. Track id: str0m picks a random one.
                        let mid = change.add_media(
                            track_in.kind,
                            Direction::SendOnly,
                            Some(track_in.user.clone()),
                            None,
                            None,
                        );
                        track.state = TrackOutState::Negotiating(mid);
                    }
                }
                TrackOutState::ToStop(mid) => {
                    change.stop_media(mid);
                    track.state = TrackOutState::NegotiatingStop(mid);
                }
                _ => {}
            }
        }

        if !change.has_changes() {
            return false;
        }
        let Some((offer, pending)) = change.apply() else {
            return false;
        };
        self.pending = Some((pending, Instant::now()));
        let _ = self.events.send(SfuEvent::Offer(offer.to_sdp_string()));
        true
    }

    pub fn handle_answer(&mut self, sdp: &str) {
        let Some((pending, _)) = self.pending.take() else {
            debug!("client {} sent an answer with no offer pending", self.user);
            return;
        };
        let result = SdpAnswer::from_sdp_string(sdp)
            .map_err(|e| e.to_string())
            .and_then(|answer| {
                self.rtc
                    .sdp_api()
                    .accept_answer(pending, answer)
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            // Our view of the m-lines no longer matches the client's: end the session.
            warn!("client {} bad answer: {e}", self.user);
            self.rtc.disconnect();
            return;
        }

        for track in &mut self.tracks_out {
            if let TrackOutState::Negotiating(m) = track.state {
                track.state = TrackOutState::Open(m);
            }
        }
        // Stopped m-lines (port 0) can be recycled by a later add_media.
        self.tracks_out
            .retain(|t| !matches!(t.state, TrackOutState::NegotiatingStop(_)));
    }

    pub fn handle_track_open(&mut self, track_in: Weak<TrackIn>) {
        // Video only for a client whose offer had video: an older build has none.
        if !self.video
            && track_in
                .upgrade()
                .is_some_and(|t| t.kind == MediaKind::Video)
        {
            return;
        }
        // A client joining while a TrackOpen is still queued subscribes from `tracks_in` at join
        // and then gets the TrackOpen too; a second m-line would be a silent duplicate stream.
        if self
            .tracks_out
            .iter()
            .any(|t| Weak::ptr_eq(&t.track_in, &track_in))
        {
            return;
        }
        self.tracks_out.push(TrackOut {
            track_in,
            state: TrackOutState::ToOpen,
        });
    }

    pub fn handle_media_data_out(&mut self, origin: ClientId, data: &MediaData) {
        let Some(mid) = self
            .tracks_out
            .iter()
            .find(|o| {
                o.track_in
                    .upgrade()
                    .is_some_and(|i| i.origin == origin && i.mid == data.mid)
            })
            .and_then(|o| o.mid())
        else {
            return;
        };
        let Some(writer) = self.rtc.writer(mid) else {
            return;
        };
        let Some(pt) = writer.match_params(data.params) else {
            return;
        };
        if let Err(e) = writer.write(pt, data.network_time, data.time, data.data.clone()) {
            warn!("client {} write failed: {e:?}", self.user);
            self.rtc.disconnect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> Client {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let rtc = Rtc::builder().build(Instant::now());
        Client::new(
            rtc,
            "r".into(),
            "b".into(),
            tx,
            vec![],
            false,
            Duration::MAX,
        )
    }

    fn track(mid: &str, kind: MediaKind) -> Arc<TrackIn> {
        Arc::new(TrackIn {
            origin: ClientId(u64::MAX),
            user: "a".into(),
            mid: Mid::from(mid),
            kind,
        })
    }

    #[test]
    fn track_open_is_idempotent() {
        let mut c = client();
        let track = track("0", MediaKind::Audio);
        // The join-time subscription and the queued TrackOpen deliver the same track.
        c.handle_track_open(Arc::downgrade(&track));
        c.handle_track_open(Arc::downgrade(&track));
        assert_eq!(c.tracks_out.len(), 1);
    }

    #[test]
    fn video_only_for_a_client_with_video() {
        let mut c = client();
        let (audio, video) = (track("0", MediaKind::Audio), track("1", MediaKind::Video));
        c.handle_track_open(Arc::downgrade(&audio));
        c.handle_track_open(Arc::downgrade(&video));
        assert_eq!(c.tracks_out.len(), 1, "an audio-only client got video");
        c.video = true;
        c.handle_track_open(Arc::downgrade(&video));
        assert_eq!(c.tracks_out.len(), 2);
    }
}
