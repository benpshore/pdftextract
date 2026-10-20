//! Retry with exponential backoff that honours `Retry-After`.
//!
//! [`RetryPolicy::delay_for`] is pure (it decides from the attempt number and
//! the error alone) and [`RetryPolicy::run`] takes the sleep function as a
//! parameter, so tests record the delays instead of waiting for them.

use std::time::Duration;

use crate::error::BiblioError;

/// How many times to retry and how long to wait between attempts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Retries after the first attempt (`0` means a single attempt).
    pub retries: u32,
    /// First backoff; doubled after every retry.
    pub base: Duration,
    /// Longest wait honoured, whether from backoff or from `Retry-After`.
    /// A server asking for more is not retried.
    pub max_wait: Duration,
}

impl Default for RetryPolicy {
    /// Four retries, 1.5 s first backoff, at most 60 s per wait.
    fn default() -> Self {
        Self {
            retries: 4,
            base: Duration::from_millis(1500),
            max_wait: Duration::from_secs(60),
        }
    }
}

impl RetryPolicy {
    /// A policy that never retries.
    pub const NONE: Self = Self {
        retries: 0,
        base: Duration::ZERO,
        max_wait: Duration::ZERO,
    };

    /// Whether `err` is worth another attempt at all: rate limits, server
    /// `Retry-After`, transport failures and 5xx statuses are; 4xx answers,
    /// bad JSON, offline mode and missing configuration are not.
    pub fn is_retryable(err: &BiblioError) -> bool {
        match err {
            BiblioError::RateLimited
            | BiblioError::RetryAfter { .. }
            | BiblioError::Transport(_) => true,
            BiblioError::Status(code) => (500..=599).contains(code),
            BiblioError::Offline
            | BiblioError::NotFound
            | BiblioError::Json(_)
            | BiblioError::MissingConfig(_)
            | BiblioError::Shape(_) => false,
        }
    }

    /// The wait before retry number `attempt` (1-based) after `err`, or
    /// `None` when the request must not be retried. A `Retry-After` beyond
    /// `max_wait` ends the retries; backoff is capped at `max_wait`.
    pub fn delay_for(&self, attempt: u32, err: &BiblioError) -> Option<Duration> {
        if attempt == 0 || attempt > self.retries || !Self::is_retryable(err) {
            return None;
        }
        let backoff = self
            .base
            .checked_mul(1u32.checked_shl(attempt - 1).unwrap_or(u32::MAX))
            .unwrap_or(self.max_wait)
            .min(self.max_wait);
        match err {
            BiblioError::RetryAfter { after, .. } => {
                if *after > self.max_wait {
                    None
                } else {
                    Some((*after).max(backoff.min(*after)))
                }
            }
            _ => Some(backoff),
        }
    }

    /// Run `request` until it succeeds, fails with a non-retryable error or
    /// the retries are spent; `sleep` receives each wait.
    pub fn run<T>(
        &self,
        mut request: impl FnMut() -> Result<T, BiblioError>,
        mut sleep: impl FnMut(Duration),
    ) -> Result<T, BiblioError> {
        let mut attempt = 0;
        loop {
            match request() {
                Ok(value) => return Ok(value),
                Err(err) => {
                    attempt += 1;
                    match self.delay_for(attempt, &err) {
                        Some(wait) => sleep(wait),
                        None => return Err(err),
                    }
                }
            }
        }
    }
}

/// Parse a `Retry-After` header value into a delay from `now_unix` (seconds
/// since the Unix epoch): either a non-negative number of seconds or an
/// HTTP date (`Wed, 21 Oct 2015 07:28:00 GMT`, also the RFC 850 and
/// asctime forms). A date in the past gives zero. Garbage gives `None`.
pub fn parse_retry_after(value: &str, now_unix: u64) -> Option<Duration> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    if v.bytes().all(|b| b.is_ascii_digit()) {
        return v.parse::<u64>().ok().map(Duration::from_secs);
    }
    let at = parse_http_date(v)?;
    Some(Duration::from_secs(at.saturating_sub(now_unix)))
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// Seconds since the Unix epoch for an HTTP date, tolerant of the three
/// forms RFC 9110 allows (IMF-fixdate, RFC 850, asctime). The weekday is
/// ignored; the zone must be absent or `GMT`/`UTC`.
pub fn parse_http_date(value: &str) -> Option<u64> {
    let cleaned = value.replace([',', '-'], " ");
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();
    let mut day: Option<u32> = None;
    let mut month: Option<u32> = None;
    let mut year: Option<i64> = None;
    let mut time: Option<(u32, u32, u32)> = None;
    for token in tokens {
        let lower = token.to_ascii_lowercase();
        if let Some(index) = MONTHS.iter().position(|m| lower.starts_with(m)) {
            if month.is_none() && lower.len() == 3 {
                month = u32::try_from(index).ok().map(|m| m + 1);
            }
        } else if token.contains(':') {
            let mut parts = token.split(':');
            let h = parts.next()?.parse::<u32>().ok()?;
            let m = parts.next()?.parse::<u32>().ok()?;
            let s = parts.next()?.parse::<u32>().ok()?;
            if parts.next().is_some() || h > 23 || m > 59 || s > 60 {
                return None;
            }
            time = Some((h, m, s));
        } else if token.bytes().all(|b| b.is_ascii_digit()) {
            match token.len() {
                1 | 2 if day.is_none() => day = Some(token.parse().ok()?),
                2 if year.is_none() => {
                    // RFC 850 two-digit year: 1970–2069 as RFC 9110 suggests.
                    let yy: i64 = token.parse().ok()?;
                    year = Some(if yy >= 70 { 1900 + yy } else { 2000 + yy });
                }
                4 => year = Some(token.parse().ok()?),
                _ => return None,
            }
        } else if !matches!(lower.as_str(), "gmt" | "utc" | "z") && !is_weekday(&lower) {
            return None;
        }
    }
    let (day, month, year, (h, m, s)) = (day?, month?, year?, time?);
    if !(1..=31).contains(&day) || !(1970..=9999).contains(&year) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let secs = days.checked_mul(86_400)? + i64::from(h) * 3600 + i64::from(m) * 60 + i64::from(s);
    u64::try_from(secs).ok()
}

fn is_weekday(lower: &str) -> bool {
    ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
        .iter()
        .any(|d| lower.starts_with(d))
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = i64::from(month);
    let d = i64::from(day);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_seconds_and_dates() {
        assert_eq!(parse_retry_after("120", 0), Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after(" 0 ", 0), Some(Duration::ZERO));
        assert_eq!(parse_retry_after("", 0), None);
        assert_eq!(parse_retry_after("soon", 0), None);
        assert_eq!(parse_retry_after("-5", 0), None);
        // 2015-10-21T07:28:00Z = 1445412480
        let fixdate = "Wed, 21 Oct 2015 07:28:00 GMT";
        assert_eq!(parse_http_date(fixdate), Some(1_445_412_480));
        assert_eq!(
            parse_retry_after(fixdate, 1_445_412_480 - 30),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            parse_retry_after(fixdate, 1_445_412_480 + 30),
            Some(Duration::ZERO)
        );
        assert_eq!(
            parse_http_date("Wednesday, 21-Oct-15 07:28:00 GMT"),
            Some(1_445_412_480)
        );
        assert_eq!(
            parse_http_date("Wed Oct 21 07:28:00 2015"),
            Some(1_445_412_480)
        );
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(parse_http_date("Wed, 21 Oct 2015 07:28 GMT"), None);
        assert_eq!(parse_http_date("Wed, 21 Oct 2015 07:28:00 PST"), None);
        assert_eq!(parse_http_date("21 Oct 2015"), None);
    }

    #[test]
    fn civil_days() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2024, 2, 29), 19_782);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let policy = RetryPolicy {
            retries: 3,
            base: Duration::from_secs(1),
            max_wait: Duration::from_secs(3),
        };
        let err = BiblioError::RateLimited;
        assert_eq!(policy.delay_for(1, &err), Some(Duration::from_secs(1)));
        assert_eq!(policy.delay_for(2, &err), Some(Duration::from_secs(2)));
        assert_eq!(policy.delay_for(3, &err), Some(Duration::from_secs(3)));
        assert_eq!(policy.delay_for(4, &err), None);
        assert_eq!(policy.delay_for(0, &err), None);
        assert_eq!(policy.delay_for(1, &BiblioError::NotFound), None);
        assert_eq!(policy.delay_for(1, &BiblioError::Offline), None);
        assert_eq!(
            policy.delay_for(1, &BiblioError::Status(503)),
            Some(Duration::from_secs(1))
        );
        assert_eq!(policy.delay_for(1, &BiblioError::Status(400)), None);
        assert_eq!(RetryPolicy::NONE.delay_for(1, &err), None);
    }

    #[test]
    fn retry_after_overrides_backoff_within_the_cap() {
        let policy = RetryPolicy {
            retries: 2,
            base: Duration::from_secs(1),
            max_wait: Duration::from_secs(10),
        };
        let asked = |secs| BiblioError::RetryAfter {
            status: 429,
            after: Duration::from_secs(secs),
        };
        assert_eq!(policy.delay_for(1, &asked(5)), Some(Duration::from_secs(5)));
        // A shorter server hint than the backoff is still honoured as asked.
        assert_eq!(policy.delay_for(2, &asked(1)), Some(Duration::from_secs(1)));
        // Too long to wait: stop.
        assert_eq!(policy.delay_for(1, &asked(11)), None);
    }

    #[test]
    fn run_records_waits_and_stops() {
        let policy = RetryPolicy {
            retries: 2,
            base: Duration::from_millis(10),
            max_wait: Duration::from_secs(1),
        };
        let mut calls = 0;
        let mut waits: Vec<Duration> = Vec::new();
        let result: Result<u8, BiblioError> = policy.run(
            || {
                calls += 1;
                if calls < 3 {
                    Err(BiblioError::Transport("timeout".into()))
                } else {
                    Ok(7)
                }
            },
            |d| waits.push(d),
        );
        assert_eq!(result.unwrap(), 7);
        assert_eq!(
            waits,
            vec![Duration::from_millis(10), Duration::from_millis(20)]
        );

        let mut calls = 0;
        let result: Result<u8, BiblioError> = policy.run(
            || {
                calls += 1;
                Err(BiblioError::RateLimited)
            },
            |_| {},
        );
        assert!(matches!(result, Err(BiblioError::RateLimited)));
        assert_eq!(calls, 3);

        let mut calls = 0;
        let result: Result<u8, BiblioError> = policy.run(
            || {
                calls += 1;
                Err(BiblioError::Offline)
            },
            |_| panic!("offline is never retried"),
        );
        assert!(matches!(result, Err(BiblioError::Offline)));
        assert_eq!(calls, 1);
    }
}
