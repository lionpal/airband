//! Small answers the app asks of the core, so it follows the same rules as his computer: what a team he names
//! would be called, whether a link pairs, and the words on the buttons that answer a request for leave.

/// The word on the button that allows a request for leave, or on the one that refuses it.
#[uniffi::export]
pub fn leave_word(allow: bool) -> String {
    if allow { airband_core::radio::ALLOW } else { airband_core::radio::DENY }.to_string()
}

/// What a team he names this way would be called: the computer names it by the same rule. Empty when the
/// words make no name.
#[uniffi::export]
pub fn team_name(written: String) -> String {
    airband_core::routing::team_name(&written).unwrap_or_default()
}

/// Is this a pairing link a computer gave (`airband pair`)? Read by the one parser that pairs with it.
#[uniffi::export]
pub fn is_pairing_link(link: String) -> bool {
    airband_core::pairing::Pairing::parse(link.trim()).is_ok()
}
