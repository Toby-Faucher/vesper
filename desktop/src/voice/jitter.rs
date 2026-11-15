//! A jitter buffer per remote voice: 20 ms Opus frames held by RTP time and played in order after a fixed delay.
//!
//! shortcut: a fixed 60 ms target, no adaptive delay and no in-band FEC; upgrade when real calls show late packets.

use std::collections::BTreeMap;

/// RTP ticks per 20 ms frame at 48 kHz.
pub const FRAME_TICKS: u64 = 960;
/// Frames held before playing starts: 60 ms.
pub const TARGET: usize = 3;
/// Past this the oldest frames are dropped, so the delay cannot build up (200 ms).
pub const MAX_HELD: usize = 10;
/// Frames concealed in a row before the buffer waits to fill again.
pub const MAX_LOST: u32 = 5;
/// A frame this far from the play point is a new stream (a rejoin, an unmute), not a late or early packet.
pub const RESET: u64 = 50;

#[derive(Debug, PartialEq, Eq)]
pub enum Pop {
    /// The next frame's Opus payload.
    Frame(Vec<u8>),
    /// The next frame is missing: conceal it (Opus PLC).
    Lost,
    /// Nothing to play.
    Silent,
}

#[derive(Debug, Default)]
pub struct Jitter {
    frames: BTreeMap<u64, Vec<u8>>,
    /// The frame to play next; `None` while filling.
    next: Option<u64>,
    lost: u32,
}

impl Jitter {
    /// A packet with RTP time `rtp` (48 kHz ticks, already unwrapped).
    pub fn push(&mut self, rtp: u64, payload: Vec<u8>) {
        let i = rtp / FRAME_TICKS;
        if let Some(n) = self.next {
            if i.abs_diff(n) > RESET {
                *self = Jitter::default();
            } else if i < n {
                return; // late: its turn was played or concealed
            }
        }
        self.frames.insert(i, payload);
        while self.frames.len() > MAX_HELD {
            self.frames.pop_first();
            if self.next.is_some() {
                self.next = self.frames.keys().next().copied();
            }
        }
    }

    /// The next 20 ms. Called once per frame time.
    pub fn pop(&mut self) -> Pop {
        let n = match self.next {
            Some(n) => n,
            None if self.frames.len() >= TARGET => *self.frames.keys().next().expect("held"),
            None => return Pop::Silent,
        };
        if let Some(p) = self.frames.remove(&n) {
            (self.next, self.lost) = (Some(n + 1), 0);
            return Pop::Frame(p);
        }
        self.lost += 1;
        if self.lost > MAX_LOST {
            (self.next, self.lost) = (None, 0);
            return Pop::Silent;
        }
        self.next = Some(n + 1);
        Pop::Lost
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(i: u64) -> Vec<u8> {
        vec![i as u8]
    }

    fn push(j: &mut Jitter, i: u64) {
        j.push(1_000_000 * FRAME_TICKS + i * FRAME_TICKS, frame(i));
    }

    fn ids(pops: Vec<Pop>) -> Vec<Option<u8>> {
        pops.into_iter()
            .map(|p| match p {
                Pop::Frame(f) => Some(f[0]),
                Pop::Lost => Some(255),
                Pop::Silent => None,
            })
            .collect()
    }

    #[test]
    fn waits_for_the_target_then_plays_in_order() {
        let mut j = Jitter::default();
        push(&mut j, 1);
        assert_eq!(j.pop(), Pop::Silent);
        push(&mut j, 0);
        assert_eq!(j.pop(), Pop::Silent);
        push(&mut j, 2);
        let got = ids((0..3).map(|_| j.pop()).collect());
        assert_eq!(got, [Some(0), Some(1), Some(2)]);
    }

    #[test]
    fn a_gap_is_concealed_then_playing_goes_on() {
        let mut j = Jitter::default();
        for i in [0, 1, 3, 4] {
            push(&mut j, i);
        }
        let got = ids((0..5).map(|_| j.pop()).collect());
        assert_eq!(got, [Some(0), Some(1), Some(255), Some(3), Some(4)]);
    }

    #[test]
    fn a_late_packet_is_dropped() {
        let mut j = Jitter::default();
        for i in 0..4 {
            push(&mut j, i);
        }
        assert_eq!(ids(vec![j.pop(), j.pop()]), [Some(0), Some(1)]);
        push(&mut j, 0);
        push(&mut j, 1);
        assert_eq!(ids(vec![j.pop(), j.pop()]), [Some(2), Some(3)]);
    }

    #[test]
    fn a_frame_arriving_before_its_turn_fills_the_gap() {
        let mut j = Jitter::default();
        for i in [0, 1, 2, 4] {
            push(&mut j, i);
        }
        assert_eq!(ids(vec![j.pop(), j.pop()]), [Some(0), Some(1)]);
        push(&mut j, 3);
        assert_eq!(
            ids(vec![j.pop(), j.pop(), j.pop()]),
            [Some(2), Some(3), Some(4)]
        );
    }

    #[test]
    fn an_underrun_conceals_a_little_then_waits_to_fill_again() {
        let mut j = Jitter::default();
        for i in 0..3 {
            push(&mut j, i);
        }
        let got = ids((0..10).map(|_| j.pop()).collect());
        let lost = vec![Some(255); MAX_LOST as usize];
        assert_eq!(got[..3], [Some(0), Some(1), Some(2)]);
        assert_eq!(got[3..3 + MAX_LOST as usize], lost[..]);
        assert_eq!(got[3 + MAX_LOST as usize], None);
        // The voice comes back later (an unmute): it starts again after the target.
        for i in 30..33 {
            push(&mut j, i);
        }
        assert_eq!(ids(vec![j.pop(), j.pop()]), [Some(30), Some(31)]);
    }

    #[test]
    fn a_new_stream_far_away_resets_and_too_much_is_dropped() {
        let mut j = Jitter::default();
        for i in 0..3 {
            push(&mut j, i);
        }
        j.pop();
        // A rejoin: the sender's clock starts somewhere else.
        j.push(7 * FRAME_TICKS, vec![7]);
        assert_eq!(j.pop(), Pop::Silent, "filling again");
        // A burst: only the newest MAX_HELD are kept.
        for i in 8..30 {
            j.push(i * FRAME_TICKS, vec![i as u8]);
        }
        assert_eq!(j.pop(), Pop::Frame(vec![20]));
    }
}
