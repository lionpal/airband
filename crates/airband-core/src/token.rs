//! Tokens on a request: reading one from its `Authorization` header and comparing it with the one wanted. The
//! tower and the relay both do this, here and nowhere else.

/// The token in an `Authorization: Bearer …` header's value. Empty when there is none, or it is not a bearer
/// token.
pub fn bearer(authorization: Option<&str>) -> &str {
    authorization.and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("")
}

/// Are these the same secret? Every byte is compared whatever the outcome, so how long it takes says nothing
/// about how much of a guess was right.
pub fn same(given: &[u8], want: &[u8]) -> bool {
    given.len() == want.len() && given.iter().zip(want).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bearer_token_is_read_only_from_a_bearer_header_and_compared_whole() {
        assert_eq!(bearer(Some("Bearer abc")), "abc");
        assert_eq!(bearer(Some("Basic abc")), "");
        assert_eq!(bearer(None), "");
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd") && !same(b"abc", b"abcd") && !same(b"", b"a"));
    }
}
