//! The outcome of one registry lookup, with an explicit reason when the
//! answer is unknown, so an offline run is reported as such and never
//! mistaken for "this identifier does not exist".

use crate::error::BiblioError;

/// What a lookup established.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup<T> {
    /// The registry answered with a record.
    Found(T),
    /// The registry answered that it has no record.
    NotFound,
    /// No answer was obtained; the string says why (`offline`, `network:
    /// timeout`, `rate limited`, `HTTP status 500`, `invalid JSON: ...`).
    Unresolved(String),
}

impl<T> Lookup<T> {
    /// Turn a client result into a lookup outcome.
    pub fn from_result(result: Result<Option<T>, BiblioError>) -> Self {
        match result {
            Ok(Some(value)) => Self::Found(value),
            Ok(None) | Err(BiblioError::NotFound) => Self::NotFound,
            Err(err) => Self::Unresolved(unresolved_reason(&err)),
        }
    }

    /// `found`, `not_found`, or `unresolved: <reason>`.
    pub fn status(&self) -> String {
        match self {
            Self::Found(_) => "found".to_string(),
            Self::NotFound => "not_found".to_string(),
            Self::Unresolved(reason) => format!("unresolved: {reason}"),
        }
    }

    /// The record, if found.
    pub fn found(self) -> Option<T> {
        match self {
            Self::Found(value) => Some(value),
            _ => None,
        }
    }

    /// True when no answer was obtained.
    pub fn is_unresolved(&self) -> bool {
        matches!(self, Self::Unresolved(_))
    }

    /// Map the found value.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Lookup<U> {
        match self {
            Self::Found(value) => Lookup::Found(f(value)),
            Self::NotFound => Lookup::NotFound,
            Self::Unresolved(reason) => Lookup::Unresolved(reason),
        }
    }
}

/// The short reason an error leaves a lookup unresolved.
pub fn unresolved_reason(err: &BiblioError) -> String {
    match err {
        BiblioError::Offline => "offline".to_string(),
        BiblioError::NotFound => "not found".to_string(),
        BiblioError::RateLimited => "rate limited".to_string(),
        BiblioError::RetryAfter { status, after } => {
            format!("HTTP status {status}, retry after {} s", after.as_secs())
        }
        BiblioError::Status(code) => format!("HTTP status {code}"),
        BiblioError::Transport(detail) => format!("network: {detail}"),
        BiblioError::Json(e) => format!("invalid JSON: {e}"),
        BiblioError::MissingConfig(what) => format!("missing configuration: {what}"),
        BiblioError::Shape(detail) => format!("unexpected response shape: {detail}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_name_the_reason() {
        assert_eq!(Lookup::Found(1).status(), "found");
        assert_eq!(Lookup::<u8>::NotFound.status(), "not_found");
        assert_eq!(
            Lookup::<u8>::from_result(Err(BiblioError::Offline)).status(),
            "unresolved: offline"
        );
        assert_eq!(
            Lookup::<u8>::from_result(Err(BiblioError::Transport("timeout".into()))).status(),
            "unresolved: network: timeout"
        );
        assert_eq!(
            Lookup::<u8>::from_result(Err(BiblioError::NotFound)),
            Lookup::NotFound
        );
        assert_eq!(Lookup::<u8>::from_result(Ok(None)), Lookup::NotFound);
        assert_eq!(Lookup::from_result(Ok(Some(3))).found(), Some(3));
        assert!(Lookup::<u8>::Unresolved("x".into()).is_unresolved());
        assert_eq!(Lookup::Found(2).map(|v| v * 2), Lookup::Found(4));
    }
}
