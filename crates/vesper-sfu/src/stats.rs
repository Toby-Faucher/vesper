//! Counters the SFU loop keeps for the server's `/metrics`: packets in and out of the media socket by kind, what was
//! dropped and why, and how many sessions and rooms are live. Plain atomics, written by the SFU thread and read by
//! [`SfuHandle::stats`](crate::SfuHandle::stats) from any thread. No per-user or per-room labels.

use std::sync::atomic::{AtomicU64, Ordering};

use str0m::media::MediaKind;

/// What a datagram on the media socket carries, as far as the SFU can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketKind {
    /// RTP whose payload type the session maps to an audio codec (Opus, or its RTX or RED).
    Audio,
    /// RTP whose payload type the session maps to a video codec (VP8, or its RTX or RED).
    Video,
    /// RTCP (sender and receiver reports, NACK, PLI, FIR, TWCC), told apart by RFC 5761.
    Rtcp,
    /// STUN, DTLS, RTP of a payload type the session does not know, or anything else.
    Other,
}

impl PacketKind {
    pub const ALL: [PacketKind; 4] = [
        PacketKind::Audio,
        PacketKind::Video,
        PacketKind::Rtcp,
        PacketKind::Other,
    ];

    /// The label value in `/metrics`.
    pub fn label(self) -> &'static str {
        match self {
            PacketKind::Audio => "audio",
            PacketKind::Video => "video",
            PacketKind::Rtcp => "rtcp",
            PacketKind::Other => "other",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Sort one datagram by its first two bytes (RFC 7983 and RFC 5761). The RTP header is not encrypted by SRTP, so
/// this works on what the socket sees. `kind_of` maps an RTP payload type to a media kind for the session the
/// datagram belongs to; `None` (no session, or an unknown type) makes it [`PacketKind::Other`].
pub(crate) fn classify(data: &[u8], kind_of: impl FnOnce(u8) -> Option<MediaKind>) -> PacketKind {
    match data {
        [128..=191, second, ..] if (192..=223).contains(second) => PacketKind::Rtcp,
        [128..=191, second, ..] => match kind_of(second & 0x7f) {
            Some(MediaKind::Audio) => PacketKind::Audio,
            Some(MediaKind::Video) => PacketKind::Video,
            None => PacketKind::Other,
        },
        _ => PacketKind::Other,
    }
}

/// Why media the SFU received was not forwarded. Audio counts one per packet, video one per frame (str0m hands the
/// SFU whole frames).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaDrop {
    /// A force-muted user's audio.
    Muted,
    /// Video from a session the server has not marked as sharing.
    NotSharing,
    /// The video frame that cut a share for staying over `SfuConfig::share_ceiling_bps`. The frames after it count
    /// as [`MediaDrop::NotSharing`].
    OverCeiling,
    /// Writing to a receiving session failed; that session is closed.
    WriteFailed,
}

impl MediaDrop {
    pub const ALL: [MediaDrop; 4] = [
        MediaDrop::Muted,
        MediaDrop::NotSharing,
        MediaDrop::OverCeiling,
        MediaDrop::WriteFailed,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MediaDrop::Muted => "muted",
            MediaDrop::NotSharing => "not_sharing",
            MediaDrop::OverCeiling => "over_ceiling",
            MediaDrop::WriteFailed => "write_failed",
        }
    }
}

/// Counts by [`PacketKind`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ByKind([u64; 4]);

impl ByKind {
    pub fn get(&self, kind: PacketKind) -> u64 {
        self.0[kind.index()]
    }
}

/// A snapshot of the SFU's counters. Counters only grow while the SFU runs; `sessions` and `rooms` are current.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SfuStats {
    /// Live peer connections, connection checks included.
    pub sessions: u64,
    /// Rooms with at least one live session. A connection check is a room of its own.
    pub rooms: u64,
    /// Datagrams read from the media socket.
    pub packets_in: ByKind,
    /// Datagrams the SFU tried to send, failed sends included.
    pub packets_out: ByKind,
    /// Datagrams no session accepted (often a client's first STUN that beats its join).
    pub dropped_no_session: ByKind,
    /// Datagrams whose `send_to` failed.
    pub dropped_send_error: ByKind,
    /// Media not forwarded, by reason: `[audio, video]` for each [`MediaDrop`] in [`MediaDrop::ALL`] order.
    pub media_dropped: [[u64; 2]; 4],
}

impl SfuStats {
    /// Media of `kind` (audio or video) not forwarded for `why`.
    pub fn media_dropped(&self, kind: MediaKind, why: MediaDrop) -> u64 {
        self.media_dropped[why as usize][media_index(kind)]
    }
}

fn media_index(kind: MediaKind) -> usize {
    match kind {
        MediaKind::Audio => 0,
        MediaKind::Video => 1,
    }
}

#[derive(Default)]
struct KindCounters([AtomicU64; 4]);

impl KindCounters {
    fn add(&self, kind: PacketKind) {
        self.0[kind.index()].fetch_add(1, Ordering::Relaxed);
    }

    fn load(&self) -> ByKind {
        ByKind(std::array::from_fn(|i| self.0[i].load(Ordering::Relaxed)))
    }
}

/// The live counters, shared between the SFU thread and its handles.
#[derive(Default)]
pub(crate) struct Counters {
    sessions: AtomicU64,
    rooms: AtomicU64,
    packets_in: KindCounters,
    packets_out: KindCounters,
    dropped_no_session: KindCounters,
    dropped_send_error: KindCounters,
    media_dropped: [[AtomicU64; 2]; 4],
}

impl Counters {
    pub fn packet_in(&self, kind: PacketKind) {
        self.packets_in.add(kind);
    }

    pub fn packet_out(&self, kind: PacketKind) {
        self.packets_out.add(kind);
    }

    pub fn no_session(&self, kind: PacketKind) {
        self.dropped_no_session.add(kind);
    }

    pub fn send_error(&self, kind: PacketKind) {
        self.dropped_send_error.add(kind);
    }

    pub fn media_dropped(&self, kind: MediaKind, why: MediaDrop) {
        self.media_dropped[why as usize][media_index(kind)].fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_live(&self, sessions: u64, rooms: u64) {
        self.sessions.store(sessions, Ordering::Relaxed);
        self.rooms.store(rooms, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> SfuStats {
        SfuStats {
            sessions: self.sessions.load(Ordering::Relaxed),
            rooms: self.rooms.load(Ordering::Relaxed),
            packets_in: self.packets_in.load(),
            packets_out: self.packets_out.load(),
            dropped_no_session: self.dropped_no_session.load(),
            dropped_send_error: self.dropped_send_error.load(),
            media_dropped: std::array::from_fn(|r| {
                std::array::from_fn(|k| self.media_dropped[r][k].load(Ordering::Relaxed))
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opus_and_vp8(pt: u8) -> Option<MediaKind> {
        match pt {
            111 | 63 => Some(MediaKind::Audio),
            96 | 97 => Some(MediaKind::Video),
            _ => None,
        }
    }

    #[test]
    fn datagrams_are_sorted_by_their_first_bytes() {
        let k = |d: &[u8]| classify(d, opus_and_vp8);
        // RTP version 2, payload type 111, with and without the marker bit.
        assert_eq!(k(&[0x80, 111, 0, 1]), PacketKind::Audio);
        assert_eq!(k(&[0x80, 0x80 | 111, 0, 1]), PacketKind::Audio);
        assert_eq!(k(&[0x90, 96, 0, 1]), PacketKind::Video);
        assert_eq!(k(&[0x80, 0x80 | 97]), PacketKind::Video);
        // RTCP: sender report (200), receiver report (201), PSFB (206, PLI and FIR), and a padded SR.
        for pt in [200, 201, 205, 206] {
            assert_eq!(k(&[0x81, pt, 0, 1]), PacketKind::Rtcp, "rtcp type {pt}");
        }
        assert_eq!(k(&[0xa0, 200]), PacketKind::Rtcp);
        // An RTP payload type the session does not map, STUN (first byte 0 or 1), DTLS (20 to 63), short data.
        assert_eq!(k(&[0x80, 8, 0, 1]), PacketKind::Other);
        assert_eq!(k(&[0x00, 0x01, 0, 0]), PacketKind::Other);
        assert_eq!(k(&[22, 0xfe, 0xfd]), PacketKind::Other);
        assert_eq!(k(&[0x80]), PacketKind::Other);
        assert_eq!(k(&[]), PacketKind::Other);
    }

    #[test]
    fn a_datagram_without_a_session_is_other_unless_it_is_rtcp() {
        assert_eq!(classify(&[0x80, 111], |_| None), PacketKind::Other);
        assert_eq!(classify(&[0x80, 200], |_| None), PacketKind::Rtcp);
    }

    #[test]
    fn the_snapshot_reads_every_counter_back() {
        let c = Counters::default();
        c.packet_in(PacketKind::Audio);
        c.packet_in(PacketKind::Audio);
        c.packet_out(PacketKind::Rtcp);
        c.no_session(PacketKind::Other);
        c.send_error(PacketKind::Video);
        c.media_dropped(MediaKind::Video, MediaDrop::NotSharing);
        c.set_live(3, 2);
        let s = c.snapshot();
        assert_eq!((s.sessions, s.rooms), (3, 2));
        assert_eq!(s.packets_in.get(PacketKind::Audio), 2);
        assert_eq!(s.packets_in.get(PacketKind::Video), 0);
        assert_eq!(s.packets_out.get(PacketKind::Rtcp), 1);
        assert_eq!(s.dropped_no_session.get(PacketKind::Other), 1);
        assert_eq!(s.dropped_send_error.get(PacketKind::Video), 1);
        assert_eq!(s.media_dropped(MediaKind::Video, MediaDrop::NotSharing), 1);
        assert_eq!(s.media_dropped(MediaKind::Audio, MediaDrop::NotSharing), 0);
    }
}
