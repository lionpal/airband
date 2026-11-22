//! Small answers the app asks of the core, so it follows the same rules as his computer: what a team he names
//! would be called, whether a link pairs, and the words on the buttons that answer a request for leave.

/// The word on the button that allows a request for leave, or on the one that refuses it.
#[uniffi::export]
pub fn leave_word(allow: bool) -> String {
    if allow { airband_core::radio::ALLOW } else { airband_core::radio::DENY }.to_string()
}

/// What a team he names this way would be called: the computer names it by the same rule
/// (`chosen_team_name`). Empty when the words make no name it would take.
#[uniffi::export]
pub fn team_name(written: String) -> String {
    airband_core::routing::chosen_team_name(&written).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_team_name_the_computer_would_refuse_is_no_name() {
        assert_eq!(super::team_name("Shop front".into()), "shop front");
        assert_eq!(super::team_name("a name far longer than any team may have".into()), "");
    }
}

/// Is this a pairing link a computer gave (`airband pair`)? Read by the one parser that pairs with it.
#[uniffi::export]
pub fn is_pairing_link(link: String) -> bool {
    airband_core::pairing::Pairing::parse(link.trim()).is_ok()
}
