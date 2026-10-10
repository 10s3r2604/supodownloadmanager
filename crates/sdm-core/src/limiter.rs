use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::time::Instant;

use crate::segment::lock;

/// A token bucket shared by every connection that should obey the same
/// speed limit.
///
/// Callers take tokens before using bandwidth; when the bucket runs dry they
/// go into debt and sleep until the debt is paid back, which spreads the
/// bandwidth evenly between connections.
#[derive(Debug)]
pub(crate) struct RateLimiter {
    /// Bytes per second, `0` means unlimited.
    limit: AtomicU64,
    bucket: Mutex<Bucket>,
}

#[derive(Debug)]
struct Bucket {
    balance: f64,
    last: Instant,
}

impl RateLimiter {
    pub(crate) fn new(limit: Option<u64>) -> Self {
        Self {
            limit: AtomicU64::new(limit.unwrap_or(0)),
            bucket: Mutex::new(Bucket {
                balance: 0.0,
                last: Instant::now(),
            }),
        }
    }

    pub(crate) fn set_limit(&self, limit: Option<u64>) {
        self.limit.store(limit.unwrap_or(0), Ordering::Relaxed);
    }

    pub(crate) fn limit(&self) -> Option<u64> {
        match self.limit.load(Ordering::Relaxed) {
            0 => None,
            limit => Some(limit),
        }
    }

    /// Waits until `bytes` may be transferred.
    pub(crate) async fn acquire(&self, bytes: usize) {
        let delay = self.reserve(bytes);
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }

    fn reserve(&self, bytes: usize) -> Duration {
        let Some(limit) = self.limit() else {
            return Duration::ZERO;
        };
        let rate = limit as f64;
        // Allow short bursts of a quarter second worth of data.
        let burst = (rate / 4.0).max(16.0 * 1024.0);

        let mut bucket = lock(&self.bucket);
        let now = Instant::now();
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.last = now;
        bucket.balance = (bucket.balance + elapsed * rate).min(burst);
        bucket.balance -= bytes as f64;
        if bucket.balance >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-bucket.balance / rate)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_never_waits() {
        let limiter = RateLimiter::new(None);
        assert_eq!(limiter.reserve(10 << 20), Duration::ZERO);
    }

    #[test]
    fn debt_turns_into_delay() {
        let limiter = RateLimiter::new(Some(1024 * 1024));
        // The bucket starts empty, so one second worth of data costs about
        // one second of waiting.
        let delay = limiter.reserve(1024 * 1024);
        assert!(delay > Duration::from_millis(700), "{delay:?}");
        assert!(delay <= Duration::from_millis(1001), "{delay:?}");
    }

    #[test]
    fn limit_can_be_changed() {
        let limiter = RateLimiter::new(Some(1000));
        assert_eq!(limiter.limit(), Some(1000));
        limiter.set_limit(None);
        assert_eq!(limiter.limit(), None);
        assert_eq!(limiter.reserve(1 << 30), Duration::ZERO);
    }
}
