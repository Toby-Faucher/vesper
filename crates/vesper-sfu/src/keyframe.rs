//! Keyframe requests (PLI or FIR) to one sharer's video, coalesced so one viewer on a bad link cannot make the
//! sharer send keyframes for everyone all the time. Ideas from LiveKit's receiver and mediasoup's
//! `KeyFrameRequestManager`; the code is our own.
//!
//! The rule: a request after a quiet interval goes out at once (a new viewer gets a picture fast). Requests inside
//! the interval after one that went out are merged into one, sent when the interval ends, so nobody's request is
//! lost and the sharer gets at most one request per interval.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use str0m::media::KeyframeRequestKind;

/// Keyframe request counts for the whole SFU, shared with [`crate::SfuHandle::keyframe_stats`].
#[derive(Debug, Default)]
pub(crate) struct KeyframeCounters {
    forwarded: AtomicU64,
    suppressed: AtomicU64,
}

impl KeyframeCounters {
    pub fn add_forwarded(&self) {
        self.forwarded.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_suppressed(&self) {
        self.suppressed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> KeyframeStats {
        KeyframeStats {
            forwarded: self.forwarded.load(Ordering::Relaxed),
            suppressed: self.suppressed.load(Ordering::Relaxed),
        }
    }
}

/// Keyframe requests since the SFU started, for every sharer together.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyframeStats {
    /// Requests sent on to a sharer. A merged request counts once.
    pub forwarded: u64,
    /// Requests that were merged into another one instead of being sent on their own.
    pub suppressed: u64,
}

/// What [`KeyframeLimiter::request`] did with a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Send it to the sharer now.
    Forward(KeyframeRequestKind),
    /// Merged: one request goes out when the interval ends ([`KeyframeLimiter::due`]).
    Suppressed,
}

/// The limiter for one sharer's video track. Time comes in as an argument, so tests drive it.
#[derive(Debug)]
pub(crate) struct KeyframeLimiter {
    interval: Duration,
    /// When the last request went to the sharer.
    last_sent: Option<Instant>,
    /// A merged request waiting for the interval to end, and its kind (FIR wins over PLI).
    pending: Option<KeyframeRequestKind>,
}

impl KeyframeLimiter {
    pub fn new(interval: Duration) -> Self {
        KeyframeLimiter {
            interval,
            last_sent: None,
            pending: None,
        }
    }

    /// A viewer (or a new watch) asks for a keyframe at `now`.
    pub fn request(&mut self, kind: KeyframeRequestKind, now: Instant) -> Verdict {
        if self.ready(now) && self.pending.is_none() {
            self.last_sent = Some(now);
            return Verdict::Forward(kind);
        }
        self.pending = Some(match (self.pending, kind) {
            (Some(KeyframeRequestKind::Fir), _) | (_, KeyframeRequestKind::Fir) => {
                KeyframeRequestKind::Fir
            }
            _ => KeyframeRequestKind::Pli,
        });
        Verdict::Suppressed
    }

    /// The merged request to send at `now`, once its interval has ended.
    pub fn due(&mut self, now: Instant) -> Option<KeyframeRequestKind> {
        if self.pending.is_none() || !self.ready(now) {
            return None;
        }
        self.last_sent = Some(now);
        self.pending.take()
    }

    /// When a merged request falls due, if one waits.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending?;
        self.last_sent.map(|t| t + self.interval)
    }

    fn ready(&self, now: Instant) -> bool {
        self.last_sent
            .is_none_or(|t| now.saturating_duration_since(t) >= self.interval)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use KeyframeRequestKind::{Fir, Pli};

    const INTERVAL: Duration = Duration::from_millis(500);

    fn ms(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn first_request_goes_out_at_once() {
        let (mut l, t0) = (KeyframeLimiter::new(INTERVAL), Instant::now());
        assert_eq!(l.request(Pli, t0), Verdict::Forward(Pli));
        assert_eq!(l.deadline(), None);
    }

    #[test]
    fn requests_inside_the_interval_merge_into_one_at_its_end() {
        let (mut l, t0) = (KeyframeLimiter::new(INTERVAL), Instant::now());
        assert_eq!(l.request(Pli, t0), Verdict::Forward(Pli));
        for t in [10, 100, 250, 499] {
            assert_eq!(l.request(Pli, ms(t0, t)), Verdict::Suppressed, "at {t} ms");
        }
        assert_eq!(l.deadline(), Some(ms(t0, 500)));
        assert_eq!(l.due(ms(t0, 499)), None);
        assert_eq!(l.due(ms(t0, 500)), Some(Pli));
        // Sent once: nothing more is due, and the next interval runs from 500 ms.
        assert_eq!(l.due(ms(t0, 2000)), None);
        assert_eq!(l.request(Pli, ms(t0, 600)), Verdict::Suppressed);
        assert_eq!(l.due(ms(t0, 1000)), Some(Pli));
    }

    #[test]
    fn a_request_after_a_quiet_interval_goes_out_at_once() {
        let (mut l, t0) = (KeyframeLimiter::new(INTERVAL), Instant::now());
        assert_eq!(l.request(Pli, t0), Verdict::Forward(Pli));
        assert_eq!(l.request(Pli, ms(t0, 500)), Verdict::Forward(Pli));
        assert_eq!(l.request(Pli, ms(t0, 5000)), Verdict::Forward(Pli));
    }

    #[test]
    fn a_waiting_request_keeps_its_turn() {
        let (mut l, t0) = (KeyframeLimiter::new(INTERVAL), Instant::now());
        l.request(Pli, t0);
        assert_eq!(l.request(Pli, ms(t0, 100)), Verdict::Suppressed);
        // The merged one was not sent yet (the loop was late): a new request joins it.
        assert_eq!(l.request(Pli, ms(t0, 700)), Verdict::Suppressed);
        assert_eq!(l.due(ms(t0, 700)), Some(Pli));
    }

    #[test]
    fn fir_wins_when_merged() {
        let (mut l, t0) = (KeyframeLimiter::new(INTERVAL), Instant::now());
        l.request(Pli, t0);
        l.request(Fir, ms(t0, 100));
        l.request(Pli, ms(t0, 200));
        assert_eq!(l.due(ms(t0, 500)), Some(Fir));
    }

    #[test]
    fn a_storm_is_bounded_by_the_interval() {
        let (mut l, t0) = (KeyframeLimiter::new(INTERVAL), Instant::now());
        let mut sent = 0;
        // 10 viewers asking every 10 ms for 10 s, the loop checking every 10 ms.
        for step in 0..1000 {
            let now = ms(t0, step * 10);
            for _ in 0..10 {
                if let Verdict::Forward(_) = l.request(Pli, now) {
                    sent += 1;
                }
            }
            sent += usize::from(l.due(now).is_some());
        }
        assert_eq!(sent, 20, "one request per 500 ms");
    }
}
