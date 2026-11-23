//! Bytes as text: hex, base64, and the random ids made from them. One place, so every part writes them the
//! same way.
use base64::Engine;

/// Bytes as lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hex back to bytes. `None` for an odd length or anything that is not hex.
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// `bytes` random bytes, as hex.
pub fn random_hex(bytes: usize) -> String {
    hex(&random(bytes))
}

fn random(bytes: usize) -> Vec<u8> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the system has no random source");
    buf
}

/// A random UUID, version 4 (RFC 9562): the version nibble is 4 and the variant bits are 10.
pub fn uuid_v4() -> String {
    let mut b = random(16);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex(&b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Standard base64, with padding.
pub fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Standard base64 back to bytes, ignoring surrounding white space.
pub fn unbase64(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(text.trim()).ok()
}

/// URL-safe base64 with no padding, as signed web tokens use.
pub fn base64_url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// URL-safe base64 with no padding, back to bytes.
pub fn unbase64_url(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text.trim()).ok()
}

/// Is this hex, of a length in `len`? How an id or a hash is checked before anything is made of it.
pub fn is_hex(s: &str, len: std::ops::RangeInclusive<usize>) -> bool {
    len.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A unit enum's word as its serde name writes it: how a kind or a state is kept in a column, logged, or put in a
/// query string.
pub fn word<T: serde::Serialize>(v: &T) -> Option<String> {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(String::from))
}

/// Percent-encoding for one part of an address, a path segment or a query's key or value: everything but
/// letters, digits and `-._~`. The one encoder every address we put together uses.
pub fn address_part(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_part_is_percent_encoded_one_way_everywhere() {
        // K-N2: the relay's reply route gave `+` for a space where the radio's routes gave `%20`
        assert_eq!(address_part("a b+c/d?é"), "a%20b%2Bc%2Fd%3F%C3%A9");
        assert_eq!(address_part("Az09-._~"), "Az09-._~");
        assert_eq!(crate::relay::Route::Reply("a b").on("c"), "/v1/channels/c/reply/a%20b");
    }

    #[test]
    fn hex_and_base64_go_there_and_back_and_a_uuid_says_its_version() {
        assert_eq!(hex(&[0, 15, 255]), "000fff");
        assert_eq!(unhex("000fff"), Some(vec![0, 15, 255]));
        assert_eq!(unhex("0f0"), None);
        assert_eq!(unhex("zz"), None);
        assert_eq!(unhex("+f"), None, "a sign is not hex");
        assert_eq!(unbase64(&base64(b"airband")), Some(b"airband".to_vec()));
        let u = uuid_v4();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
    }
}
