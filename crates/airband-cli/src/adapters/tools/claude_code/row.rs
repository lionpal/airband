//! What Claude Code's row says, as this code asks it, and its account read the way the row says to.
use super::HARNESS;
use crate::adapters::accounts::tests::{NOTHING, folder, looking};
use crate::adapters::{identity_in_copy, identity_of, row_says};
use airband_core::identity::AccountIdentity;
use serde_json::json;

#[test]
fn every_row_fact_this_code_asks_for_is_there() {
    // an empty answer would be a made-up value (an empty home is his home folder itself)
    for fact in ["home", "home_env", "pid_env", "entry_env", "inbox_env", "inbox_token_env", "inbox_scheme", "artifact_link", "headless_entry", "server_tool", "account_file", "account_seed", "account_email", "accounts_api", "accounts_token", "beta_header", "keychain_service", "login_key", "oauth_beta", "oauth_client", "program", "hooks_file", "sessions", "projects", "subagent_transcript", "session_kind", "session_entry"] {
        assert!(!row_says(HARNESS, fact).is_empty(), "its row says nothing for {fact}");
    }
    // a sign-in is started by the arguments the row lists, none written in the code
    assert_eq!(super::accounts::sign_in_args().unwrap(), ["auth", "login"]);
}

#[test]
fn its_account_is_its_account_id_in_its_organisation() {
    let s = airband_core::identity::sources(&crate::adapters::tool_row(HARNESS).unwrap().with).unwrap();
    let doc = json!({"oauthAccount": {"accountUuid": "acc-1", "organizationUuid": "org-1", "emailAddress": "x"}});
    assert_eq!(s[0].identity(&doc, None), Some(AccountIdentity::Id { id: "acc-1".into(), org: Some("org-1".into()) }));
    assert_eq!(s[0].identity(&json!({}), None), None);
    assert!(matches!(s[1].identity(&json!("sk-ant-fake-key-xyz9"), None), Some(AccountIdentity::Key { last4, .. }) if last4 == "xyz9"));
}

#[test]
fn a_signed_in_account_is_known_by_the_id_its_tool_wrote_and_a_key_by_its_fingerprint() {
    let home = folder();
    // nothing there: nothing known, never a guess
    assert_eq!(identity_of(HARNESS, &looking(&home, NOTHING, NOTHING)), AccountIdentity::None);
    // a key in the tool's environment, with no account record: its fingerprint, and its last four to show
    let env = |k: &str| (k == "ANTHROPIC_API_KEY").then(|| "fake-key-0000-wxyz".to_string());
    assert_eq!(identity_of(HARNESS, &looking(&home, &env, NOTHING)), AccountIdentity::of_key("anthropic", "fake-key-0000-wxyz"));
    // the account record wins over the key: the first source that gives anything
    std::fs::write(home.join(".claude.json"), r#"{"oauthAccount":{"accountUuid":"person-1","organizationUuid":"org-1","emailAddress":"x"}}"#).unwrap();
    assert_eq!(identity_of(HARNESS, &looking(&home, &env, NOTHING)), AccountIdentity::Id { id: "person-1".into(), org: Some("org-1".into()) });
    // a saved copy of that record is read the same way
    let saved = json!({ "email": "x", "oauthAccount": { "accountUuid": "person-2", "organizationUuid": "org-1" } });
    assert_eq!(identity_in_copy(HARNESS, ".claude.json", &saved), AccountIdentity::Id { id: "person-2".into(), org: Some("org-1".into()) });
    assert_eq!(identity_in_copy(HARNESS, "another.json", &saved), AccountIdentity::None, "only sources in a file of that name");
    let _ = std::fs::remove_dir_all(home);
}
