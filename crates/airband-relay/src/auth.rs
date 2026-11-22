//! Who is asking: the tokens and names on a request, and the answers for one that may not.
use super::*;

pub(super) fn same(a: &str, b: &str) -> bool {
    airband_core::token::same(a.as_bytes(), b.as_bytes())
}

/// A header's value as text, when the request has it and it is text. The one way a header is read here.
pub(super) fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

pub(super) fn bearer(headers: &HeaderMap) -> &str {
    airband_core::token::bearer(header(headers, "authorization"))
}

pub(super) fn valid_channel(c: &str) -> bool {
    airband_core::encode::is_hex(c, 32..=32)
}

pub(super) fn say(code: StatusCode, why: &str) -> Response {
    (code, why.to_string()).into_response()
}

/// A token's hash as a tower sends it: 64 hex characters.
pub(super) fn radio_hash_given(headers: &HeaderMap) -> Option<&str> {
    header(headers, RADIO_HASH_HEADER).filter(|h| airband_core::encode::is_hex(h, 64..=64))
}

/// What a radio's request gets when it may not use the channel. The same whether the token is missing or wrong
/// or there is no such channel, so asking tells a stranger nothing about which channels exist.
pub(super) fn not_the_radio() -> Response {
    say(StatusCode::UNAUTHORIZED, "not this channel's radio")
}
