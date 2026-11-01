//! In-process token-bucket rate limits. Per process only, which is fine for one binary.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Key: ip and username.
    Login,
    /// Key: ip.
    LoginIp,
    /// Key: ip.
    Invite,
    /// Using a password reset code. Key: ip.
    Reset,
    /// Making a password reset link. Key: user id.
    ResetCreate,
    /// Key: user id.
    Message,
    /// Key: user id.
    Upload,
    /// Key: user id and room id.
    Typing,
    /// Key: user id.
    Search,
    /// Edits, reactions and pins, shared. Key: user id.
    Change,
}

impl Kind {
    /// (burst, time to refill one token)
    fn budget(self) -> (f64, Duration) {
        let s = Duration::from_secs;
        match self {
            Kind::Login => (5.0, s(60)),
            Kind::LoginIp => (20.0, s(15)),
            Kind::Invite => (10.0, s(6 * 60)),
            Kind::Reset => (10.0, s(6 * 60)),
            Kind::ResetCreate => (10.0, s(6 * 60)),
            Kind::Message => (10.0, s(1)),
            Kind::Upload => (10.0, s(30)),
            Kind::Typing => (1.0, s(3)),
            Kind::Search => (10.0, s(1)),
            Kind::Change => (20.0, s(1)),
        }
    }
}

const PRUNE_ABOVE: usize = 10_000;
/// Past the cap, prune at most this often, so the O(n) sweep does not run on every call.
const PRUNE_EVERY: Duration = Duration::from_secs(60);

/// One token bucket. [`Limiter`] keeps one per key; a socket keeps its own (`ws.rs`).
pub struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    pub fn full(burst: f64, now: Instant) -> Bucket {
        Bucket {
            tokens: burst,
            at: now,
        }
    }

    /// Takes one token, refilling one per `per` up to `burst`, or returns how long until one is free.
    pub fn take(&mut self, burst: f64, per: Duration, now: Instant) -> Result<(), Duration> {
        let elapsed = now.saturating_duration_since(self.at);
        self.tokens = (self.tokens + elapsed.as_secs_f64() / per.as_secs_f64()).min(burst);
        self.at = now.max(self.at);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            Err(per.mul_f64(1.0 - self.tokens))
        }
    }
}

#[derive(Default)]
struct Buckets {
    map: HashMap<(Kind, String), Bucket>,
    pruned: Option<Instant>,
    /// Added to the clock by [`Limiter::now`]; only tests move it.
    skew: Duration,
}

#[derive(Clone, Default)]
pub struct Limiter(Arc<Mutex<Buckets>>);

impl Limiter {
    /// The limiter's clock: `Instant::now()` plus what [`Limiter::advance`] added.
    pub fn now(&self) -> Instant {
        Instant::now() + self.0.lock().unwrap_or_else(PoisonError::into_inner).skew
    }

    /// Moves [`Limiter::now`] forward, so a test can wait for a refill without sleeping.
    /// Only tests may call this: it moves the clock of every rate limit. It is `pub`, not
    /// `cfg(test)`, because the integration tests in `tests/` are a separate crate.
    #[doc(hidden)]
    pub fn advance(&self, by: Duration) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).skew += by;
    }

    /// Takes one token for `key`, or returns how long until one is free.
    pub fn check(&self, kind: Kind, key: &str, now: Instant) -> Result<(), Duration> {
        let (burst, per) = kind.budget();
        let mut guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let Buckets { map, pruned, .. } = &mut *guard;
        let due = pruned.is_none_or(|t| now.saturating_duration_since(t) >= PRUNE_EVERY);
        if map.len() > PRUNE_ABOVE && due {
            // Only drop buckets that have refilled: a fresh one would be identical.
            map.retain(|(k, _), b| {
                let (burst, per) = k.budget();
                now.saturating_duration_since(b.at) < per.mul_f64(burst)
            });
            *pruned = Some(now);
        }
        map.entry((kind, key.to_string()))
            .or_insert(Bucket::full(burst, now))
            .take(burst, per, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_burst_then_refill() {
        let l = Limiter::default();
        let now = Instant::now();
        for _ in 0..5 {
            l.check(Kind::Login, "1.2.3.4 toby", now).unwrap();
        }
        let wait = l.check(Kind::Login, "1.2.3.4 toby", now).unwrap_err();
        assert!(
            wait > Duration::from_secs(59) && wait <= Duration::from_secs(60),
            "{wait:?}"
        );
        l.check(Kind::Login, "1.2.3.4 alice", now).unwrap();
        l.check(Kind::LoginIp, "1.2.3.4 toby", now).unwrap();
        let later = now + Duration::from_secs(60);
        l.check(Kind::Login, "1.2.3.4 toby", later).unwrap();
        assert!(l.check(Kind::Login, "1.2.3.4 toby", later).is_err());
    }

    #[test]
    fn typing_is_one_per_three_seconds() {
        let l = Limiter::default();
        let now = Instant::now();
        l.check(Kind::Typing, "u r", now).unwrap();
        let wait = l
            .check(Kind::Typing, "u r", now + Duration::from_secs(1))
            .unwrap_err();
        assert!(
            wait.abs_diff(Duration::from_secs(2)) < Duration::from_millis(1),
            "{wait:?}"
        );
        l.check(Kind::Typing, "u r", now + Duration::from_secs(3))
            .unwrap();
    }

    #[test]
    fn only_refilled_keys_are_pruned_past_the_cap() {
        let l = Limiter::default();
        let now = Instant::now();
        // A drained Invite bucket refills in 60 min: it must survive a prune 10 min later.
        for _ in 0..10 {
            l.check(Kind::Invite, "1.2.3.4", now).unwrap();
        }
        for i in 0..PRUNE_ABOVE {
            l.check(Kind::Message, &i.to_string(), now).unwrap();
        }
        let later = now + Duration::from_secs(10 * 60);
        l.check(Kind::Message, "fresh", later).unwrap();
        assert_eq!(l.0.lock().unwrap().map.len(), 2);
        assert!(l.check(Kind::Invite, "1.2.3.4", later).is_ok());
        assert!(l.check(Kind::Invite, "1.2.3.4", later).is_err());
    }

    #[test]
    fn prune_runs_at_most_once_per_interval() {
        let l = Limiter::default();
        let now = Instant::now();
        for i in 0..=PRUNE_ABOVE {
            l.check(Kind::Typing, &i.to_string(), now).unwrap();
        }
        l.check(Kind::Typing, "a", now + Duration::from_secs(5))
            .unwrap();
        // Typing refills in 3 s, but the prune just ran, so these stay for now.
        for i in 0..=PRUNE_ABOVE {
            l.check(
                Kind::Typing,
                &format!("b{i}"),
                now + Duration::from_secs(10),
            )
            .unwrap();
        }
        assert!(l.0.lock().unwrap().map.len() > PRUNE_ABOVE);
        l.check(Kind::Typing, "c", now + Duration::from_secs(70))
            .unwrap();
        assert_eq!(l.0.lock().unwrap().map.len(), 1);
    }
}
