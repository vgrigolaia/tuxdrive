use std::time::Duration;

use rand::Rng;
use tracing::warn;

/// Truncated exponential backoff retry policy.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub jitter: bool,
}

impl RetryPolicy {
    /// Sensible production defaults: up to 8 attempts, 1 s → 512 s cap, with jitter.
    pub fn default_policy() -> Self {
        Self {
            max_attempts: 8,
            base_delay_ms: 1000,
            max_delay_ms: 512_000,
            jitter: true,
        }
    }

    /// Compute the sleep duration for attempt `n` (0-indexed).
    ///
    /// Formula: `min(base * 2^n, max)` + optional uniform jitter in `0..delay/4`.
    pub fn delay_for(&self, attempt: u32) -> Duration {
        // 2^attempt, capped so u64 never overflows (beyond shift 63 the result
        // is always larger than any realistic max_delay_ms anyway).
        let shift = attempt.min(63);
        let multiplier = 1u64.checked_shl(shift).unwrap_or(u64::MAX);
        let exponential = self.base_delay_ms.saturating_mul(multiplier);
        let capped = exponential.min(self.max_delay_ms);

        let jitter_ms = if self.jitter && capped > 0 {
            rand::thread_rng().gen_range(0..=capped / 4)
        } else {
            0
        };

        Duration::from_millis(capped + jitter_ms)
    }

    /// Return `true` if there are remaining attempts after `attempt` failures.
    ///
    /// `attempt` is 0-indexed, so `attempt == 0` means the first attempt just
    /// failed.
    pub fn should_retry(&self, attempt: u32) -> bool {
        attempt < self.max_attempts
    }
}

// ---------------------------------------------------------------------------
// Generic retry helper
// ---------------------------------------------------------------------------

/// Execute an async closure with retry logic.
///
/// On every failure the error is logged at `WARN` level and the task sleeps
/// for `policy.delay_for(attempt)` before the next try.  After
/// `policy.max_attempts` failures the last error is returned to the caller.
pub async fn with_retry<T, E, F, Fut>(policy: &RetryPolicy, op_name: &str, mut f: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempt = 0u32;

    loop {
        match f().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                warn!(
                    op = op_name,
                    attempt = attempt + 1,
                    max = policy.max_attempts,
                    error = %err,
                    "operation failed",
                );

                if !policy.should_retry(attempt + 1) {
                    return Err(err);
                }

                let delay = policy.delay_for(attempt);
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_increases_exponentially() {
        let policy = RetryPolicy {
            max_attempts: 8,
            base_delay_ms: 1000,
            max_delay_ms: 512_000,
            jitter: false,
        };

        assert_eq!(policy.delay_for(0), Duration::from_millis(1000));
        assert_eq!(policy.delay_for(1), Duration::from_millis(2000));
        assert_eq!(policy.delay_for(2), Duration::from_millis(4000));
        assert_eq!(policy.delay_for(9), Duration::from_millis(512_000)); // capped
    }

    #[test]
    fn should_retry_boundary() {
        let policy = RetryPolicy::default_policy();
        assert!(policy.should_retry(0));
        assert!(policy.should_retry(7));
        assert!(!policy.should_retry(8));
    }

    #[tokio::test]
    async fn with_retry_succeeds_on_third_attempt() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let calls = Arc::new(AtomicU32::new(0));
        let calls2 = calls.clone();

        let policy = RetryPolicy {
            max_attempts: 5,
            base_delay_ms: 1,
            max_delay_ms: 10,
            jitter: false,
        };

        let result: Result<&str, String> = with_retry(&policy, "test-op", || {
            let c = calls2.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Err(format!("attempt {n} failed"))
                } else {
                    Ok("ok")
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn with_retry_exhausts_attempts() {
        let policy = RetryPolicy {
            max_attempts: 3,
            base_delay_ms: 1,
            max_delay_ms: 10,
            jitter: false,
        };

        let result: Result<(), String> =
            with_retry(&policy, "always-fail", || async { Err("boom".to_string()) }).await;

        assert!(result.is_err());
    }
}
