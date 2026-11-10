//! Incoming bit rate of one stream, measured in windows of at least a second.

use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(1);

pub(crate) struct RateMeter {
    /// Start of the open window and the bytes counted in it.
    window: Option<(Instant, usize)>,
}

impl RateMeter {
    pub fn new() -> Self {
        RateMeter { window: None }
    }

    /// Count `bytes` arriving at `now`. Returns the window's bits per second when this closes a
    /// window of at least a second; the next window starts at `now`, empty.
    pub fn add(&mut self, bytes: usize, now: Instant) -> Option<u64> {
        let (start, total) = self.window.get_or_insert((now, 0));
        *total += bytes;
        let elapsed = now.saturating_duration_since(*start);
        if elapsed < WINDOW {
            return None;
        }
        let bps = *total as f64 * 8.0 / elapsed.as_secs_f64();
        self.window = Some((now, 0));
        Some(bps as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_is_reported_after_a_second() {
        let (mut m, t0) = (RateMeter::new(), Instant::now());
        assert_eq!(m.add(500_000, t0), None);
        assert_eq!(m.add(500_000, t0 + Duration::from_secs(1)), Some(8_000_000));
    }

    #[test]
    fn no_rate_inside_a_second() {
        let (mut m, t0) = (RateMeter::new(), Instant::now());
        assert_eq!(m.add(500_000, t0), None);
        assert_eq!(m.add(500_000, t0 + Duration::from_millis(999)), None);
    }
}
