//! Retry and backoff decisions (pure; the client sleeps).
//!
//! The Web API asks clients to slow down in two ways:
//! * a `Backoff: <seconds>` header on any response: make no request to the
//!   API for that long ([`backoff_delay`]);
//! * `429 Too Many Requests` with `Retry-After: <seconds>`: wait, then retry
//!   the same request ([`retry_delay`]).
//!
//! `5xx` answers and transport failures are retried with exponential delays
//! (`base_delay * 2^(attempt-1)`, capped). Everything else (`4xx` other than
//! `429`, parse errors, offline mode) is final.

use std::time::Duration;

use crate::error::ZError;

/// How often and how long to wait before retrying a failed request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts, including the first (`1` disables retries).
    pub max_attempts: u32,
    /// Delay after the first failure without `Retry-After`.
    pub base_delay: Duration,
    /// Upper bound for every wait, whether computed or server-requested.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    /// Three attempts, 1 s then 2 s, never more than 60 s.
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
        }
    }
}

impl RetryPolicy {
    /// A policy that never retries (one attempt).
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            ..Self::default()
        }
    }

    /// `base_delay * 2^(attempt-1)`, capped at `max_delay`.
    pub fn exponential(&self, attempt: u32) -> Duration {
        let factor = 2u32.saturating_pow(attempt.saturating_sub(1).min(16));
        self.base_delay.saturating_mul(factor).min(self.max_delay)
    }
}

/// How long to wait before retrying after `error`, given that `attempt`
/// attempts have been made so far (`1` after the first failure). `None`
/// means do not retry.
pub fn retry_delay(error: &ZError, attempt: u32, policy: &RetryPolicy) -> Option<Duration> {
    if attempt >= policy.max_attempts {
        return None;
    }
    match error {
        ZError::RateLimited {
            retry_after_secs: Some(secs),
        } => Some(Duration::from_secs(*secs).min(policy.max_delay)),
        ZError::RateLimited {
            retry_after_secs: None,
        }
        | ZError::Transport(_) => Some(policy.exponential(attempt)),
        ZError::Status { code, .. } if (500..=599).contains(code) => {
            Some(policy.exponential(attempt))
        }
        _ => None,
    }
}

/// The pause requested by a `Backoff` header (seconds), capped at `max`;
/// `None` for a missing or zero header.
pub fn backoff_delay(backoff_secs: Option<u64>, max: Duration) -> Option<Duration> {
    backoff_secs
        .filter(|secs| *secs > 0)
        .map(|secs| Duration::from_secs(secs).min(max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_honours_retry_after_up_to_the_cap() {
        let policy = RetryPolicy::default();
        let limited = ZError::RateLimited {
            retry_after_secs: Some(7),
        };
        assert_eq!(
            retry_delay(&limited, 1, &policy),
            Some(Duration::from_secs(7))
        );
        let long = ZError::RateLimited {
            retry_after_secs: Some(3600),
        };
        assert_eq!(
            retry_delay(&long, 1, &policy),
            Some(Duration::from_secs(60))
        );
        let unknown = ZError::RateLimited {
            retry_after_secs: None,
        };
        assert_eq!(
            retry_delay(&unknown, 2, &policy),
            Some(Duration::from_secs(2))
        );
        // The third attempt was the last one.
        assert_eq!(retry_delay(&limited, 3, &policy), None);
        assert_eq!(retry_delay(&limited, 1, &RetryPolicy::none()), None);
    }

    #[test]
    fn server_and_transport_errors_retry_others_do_not() {
        let policy = RetryPolicy::default();
        let server = ZError::Status {
            code: 503,
            message: String::new(),
        };
        assert_eq!(
            retry_delay(&server, 1, &policy),
            Some(Duration::from_secs(1))
        );
        assert_eq!(
            retry_delay(&ZError::Transport("reset".to_string()), 2, &policy),
            Some(Duration::from_secs(2))
        );
        assert_eq!(retry_delay(&ZError::Forbidden, 1, &policy), None);
        assert_eq!(retry_delay(&ZError::PreconditionFailed, 1, &policy), None);
        assert_eq!(
            retry_delay(
                &ZError::Status {
                    code: 400,
                    message: String::new()
                },
                1,
                &policy
            ),
            None
        );
    }

    #[test]
    fn exponential_delays_are_capped() {
        let policy = RetryPolicy {
            max_attempts: 10,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(3),
        };
        assert_eq!(policy.exponential(1), Duration::from_millis(500));
        assert_eq!(policy.exponential(3), Duration::from_secs(2));
        assert_eq!(policy.exponential(9), Duration::from_secs(3));
        assert_eq!(policy.exponential(40), Duration::from_secs(3));
    }

    #[test]
    fn backoff_header_delays() {
        let max = Duration::from_secs(60);
        assert_eq!(backoff_delay(Some(30), max), Some(Duration::from_secs(30)));
        assert_eq!(backoff_delay(Some(600), max), Some(max));
        assert_eq!(backoff_delay(Some(0), max), None);
        assert_eq!(backoff_delay(None, max), None);
    }
}
