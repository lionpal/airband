//! What Antigravity CLI's row says, as this code asks it, and its account read the way the row says to.
use super::NAME;
use crate::adapters::accounts::tests::{NOTHING, folder, looking};
use crate::adapters::{identity_of, row_says, tool_row};
use airband_core::identity::AccountIdentity;

#[test]
fn every_row_fact_this_code_asks_for_is_there() {
    for fact in ["home", "state", "program", "hooks_file", "presence", "conversations", "brain", "transcript", "last_conversations"] {
        assert!(!row_says(NAME, fact).is_empty(), "its row says nothing for {fact}");
    }
    // where it keeps its files, by the same rule as every tool, from its own row
    assert_eq!(crate::adapters::home_given(NAME, None), crate::config::home().join(".gemini"));
    // an agent the tower starts is held through the Agent Client Protocol, and says for itself what it runs on
    let launcher = &tool_row(NAME).unwrap().launchers[0];
    assert!(launcher.says_its_settings());
    // what it is offered on before its first agent has said for itself: Gemini 4 Neon, at each of its
    // thinking levels, by Antigravity's own ids for them (`agy acp` 1.5.15: the `model` and `thought_level`
    // settings of a session)
    let offers = launcher.offers();
    assert_eq!(offers.models.iter().map(|c| (c.name.as_str(), c.id.as_str())).collect::<Vec<_>>(), [("Gemini 4 Neon", "gemini-4-neon")]);
    assert_eq!(offers.efforts.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["low", "medium", "high", "xhigh"]);
}

#[test]
fn its_account_is_the_gemini_key_it_is_given() {
    let home = folder();
    let key = |k: &str| (k == "GEMINI_API_KEY").then(|| "fake-gemini-4321".to_string());
    assert_eq!(identity_of(NAME, &looking(&home, &key, NOTHING)), AccountIdentity::of_key("gemini", "fake-gemini-4321"));
    // a Google sign-in is kept in the system's keychain, which is never read for it: no account is made up
    assert_eq!(identity_of(NAME, &looking(&home, NOTHING, NOTHING)), AccountIdentity::None);
    let _ = std::fs::remove_dir_all(home);
}
