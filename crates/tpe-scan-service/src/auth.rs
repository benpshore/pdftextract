//! Per-launch bearer token: generated from the operating system's random
//! source, printed once by the binary, compared in constant time.

/// Bytes of entropy in a generated token (64 hex characters).
pub const TOKEN_BYTES: usize = 32;
/// Shortest token the service accepts in its configuration.
pub const MIN_TOKEN_CHARS: usize = 16;

/// A fresh token from the OS random source, hex-encoded.
pub fn generate_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0_u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes)?;
    Ok(hex(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

/// Constant-time equality of the expected and the presented token. Length
/// is not secret (every generated token has the same length).
pub fn token_matches(expected: &str, presented: &str) -> bool {
    let expected = expected.as_bytes();
    let presented = presented.as_bytes();
    if expected.is_empty() || expected.len() != presented.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (left, right) in expected.iter().zip(presented) {
        difference |= left ^ right;
    }
    difference == 0
}

/// The token of an `Authorization: Bearer <token>` header value.
pub fn bearer(value: &str) -> Option<&str> {
    let (scheme, rest) = value.trim().split_once(char::is_whitespace)?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = rest.trim();
    (!token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_hex_and_distinct() {
        let first = generate_token().unwrap();
        let second = generate_token().unwrap();
        assert_eq!(first.len(), TOKEN_BYTES * 2);
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn comparison_rejects_prefixes_and_empty_tokens() {
        assert!(token_matches("abcdef", "abcdef"));
        assert!(!token_matches("abcdef", "abcde"));
        assert!(!token_matches("abcdef", "abcdeg"));
        assert!(!token_matches("", ""));
    }

    #[test]
    fn bearer_scheme_is_case_insensitive_and_trimmed() {
        assert_eq!(bearer("Bearer abc"), Some("abc"));
        assert_eq!(bearer("  bearer   abc  "), Some("abc"));
        assert_eq!(bearer("Basic abc"), None);
        assert_eq!(bearer("Bearer"), None);
        assert_eq!(bearer("Bearer   "), None);
    }
}
