//! The link's token bucket: one per [`crate::Mux`], shared by every stream on it.

use std::time::{Duration, Instant};

use ubiq_proto::tunnel::TUNNEL_CHUNK_MAX;

/// A deficit bucket: a reader waits until the balance is positive, reads, then debits what it
/// actually read — so a short read costs what it was, and the wait happens before the socket is
/// read rather than after.
pub(crate) struct Bucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    last: Instant,
}

impl Bucket {
    pub(crate) fn new(rate_bps: u64) -> Self {
        let rate = rate_bps.max(1) as f64;
        // A tenth of a second of traffic, never less than one chunk.
        let burst = (rate / 10.0).max(TUNNEL_CHUNK_MAX as f64);
        Self {
            rate,
            burst,
            tokens: burst,
            last: Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        self.last = now;
    }

    /// How long until a read may happen; zero is now.
    pub(crate) fn delay(&mut self) -> Duration {
        self.refill();
        if self.tokens > 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64((-self.tokens + 1.0) / self.rate)
        }
    }

    pub(crate) fn debit(&mut self, bytes: usize) {
        self.refill();
        self.tokens -= bytes as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_debt_is_a_wait_and_the_burst_is_a_ceiling() {
        let mut b = Bucket::new(100_000);
        assert_eq!(b.delay(), Duration::ZERO);
        b.debit(TUNNEL_CHUNK_MAX + 50_000);
        let d = b.delay();
        // 50 000 bytes in debt at 100 000 B/s is about half a second.
        assert!(
            d > Duration::from_millis(400) && d < Duration::from_millis(600),
            "{d:?}"
        );
    }
}
