//! A per-agent token bucket, kept in memory. Board-scale traffic does not
//! justify anything more durable: a server restart simply refills everyone.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

struct Bucket {
    tokens: f64,
    last: Instant,
}

pub struct RateLimiter {
    buckets: Mutex<HashMap<i64, Bucket>>,
    capacity: f64,
    per_sec: f64,
}

impl RateLimiter {
    /// `per_minute == 0` disables limiting entirely.
    pub fn per_minute(per_minute: u32) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            capacity: f64::from(per_minute),
            per_sec: f64::from(per_minute) / 60.0,
        }
    }

    pub fn disabled(&self) -> bool {
        self.capacity <= 0.0
    }

    /// Take one token for `key`. Returns false when the caller is over budget.
    pub fn check(&self, key: i64) -> bool {
        if self.disabled() {
            return true;
        }
        // A poisoned lock only means some other task panicked mid-update; the
        // bucket map is still coherent, so carry on rather than kill the request.
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        let bucket = buckets.entry(key).or_insert(Bucket {
            tokens: self.capacity,
            last: now,
        });
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.per_sec).min(self.capacity);
        bucket.last = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RateLimiter;

    #[test]
    fn spends_the_bucket_then_refuses() {
        let rl = RateLimiter::per_minute(3);
        assert!(rl.check(1));
        assert!(rl.check(1));
        assert!(rl.check(1));
        assert!(!rl.check(1));
        // Buckets are per key.
        assert!(rl.check(2));
    }

    #[test]
    fn zero_means_unlimited() {
        let rl = RateLimiter::per_minute(0);
        for _ in 0..1000 {
            assert!(rl.check(1));
        }
    }

    #[test]
    fn refills_over_time() {
        let rl = RateLimiter::per_minute(60); // one token per second
        assert!(rl.check(1));
        {
            let mut b = rl.buckets.lock().unwrap();
            let bucket = b.get_mut(&1).unwrap();
            bucket.tokens = 0.0;
            bucket.last -= std::time::Duration::from_secs(2);
        }
        assert!(rl.check(1));
    }
}
