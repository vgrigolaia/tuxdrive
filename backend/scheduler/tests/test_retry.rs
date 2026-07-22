/// Integration tests for the retry / backoff policy.
///
/// Run with:
///   cargo test --test test_retry
use tuxdrive_scheduler::{with_retry, RetryPolicy};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

#[test]
fn delay_increases_exponentially() {
    let policy = RetryPolicy {
        max_attempts: 8,
        base_delay_ms: 1000,
        max_delay_ms: 512_000,
        jitter: false,
    };

    // delay_for(0) == 1000 ms, delay_for(1) == 2000 ms, …
    assert_eq!(policy.delay_for(0).as_millis(), 1000);
    assert_eq!(policy.delay_for(1).as_millis(), 2000);
    assert_eq!(policy.delay_for(2).as_millis(), 4000);
    assert_eq!(policy.delay_for(9).as_millis(), 512_000); // capped at max
}

#[test]
fn should_retry_within_limit() {
    let policy = RetryPolicy::default_policy();
    assert!(policy.should_retry(0));
    assert!(policy.should_retry(7));
    assert!(!policy.should_retry(8)); // exhausted
}

#[tokio::test]
async fn with_retry_succeeds_first_try() {
    let policy = RetryPolicy {
        max_attempts: 3,
        base_delay_ms: 1,
        max_delay_ms: 10,
        jitter: false,
    };

    let result: Result<i32, &str> = with_retry(&policy, "succeed_first", || async {
        Ok(42)
    })
    .await;

    assert_eq!(result.unwrap(), 42);
}

#[tokio::test]
async fn with_retry_succeeds_on_second_attempt() {
    let policy = RetryPolicy {
        max_attempts: 3,
        base_delay_ms: 1,
        max_delay_ms: 10,
        jitter: false,
    };
    let attempts = Arc::new(AtomicU32::new(0));

    let counter = Arc::clone(&attempts);
    let result: Result<&str, &str> = with_retry(&policy, "second_try", move || {
        let c = Arc::clone(&counter);
        async move {
            let n = c.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Err("first attempt fails")
            } else {
                Ok("success")
            }
        }
    })
    .await;

    assert_eq!(result.unwrap(), "success");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn with_retry_exhausts_and_returns_last_error() {
    let policy = RetryPolicy {
        max_attempts: 3,
        base_delay_ms: 1,
        max_delay_ms: 10,
        jitter: false,
    };
    let attempts = Arc::new(AtomicU32::new(0));

    let counter = Arc::clone(&attempts);
    let result: Result<(), &str> = with_retry(&policy, "always_fail", move || {
        let c = Arc::clone(&counter);
        async move {
            c.fetch_add(1, Ordering::SeqCst);
            Err("always fails")
        }
    })
    .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "always fails");
    // Should have been called max_attempts times.
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}
