//! What OpenCode's row says, as this code asks it, and its account read the way the row says to.
use super::NAME;
use crate::adapters::accounts::tests::{NOTHING, folder, looking};
use crate::adapters::{identity_of, row_says, tool_row};
use airband_core::identity::AccountIdentity;
use serde_json::json;

#[test]
fn every_row_fact_this_code_asks_for_is_there() {
    for fact in ["home", "home_env", "service", "user", "state"] {
        assert!(!row_says(NAME, fact).is_empty(), "its row says nothing for {fact}");
    }
    // where it keeps its files, by the same rule as every tool, from its own row
    assert_eq!(crate::adapters::home_given(NAME, None), crate::config::home().join(".local/state"));
}

#[test]
fn its_keys_are_known_by_the_provider_each_row_names() {
    let s = airband_core::identity::sources(&tool_row(NAME).unwrap().with).unwrap();
    assert_eq!(s[0].identity(&json!({"type": "key", "key": "k-1234"}), Some("opencode-go")), Some(AccountIdentity::of_key("opencode-go", "k-1234")));
    // a sign-in that is not a key records no account here
    assert_eq!(s[0].identity(&json!({"type": "oauth", "refresh": "r"}), Some("p")), None);
}

#[test]
fn a_table_of_credentials_gives_its_one_key_by_its_rows_provider_and_nothing_when_it_holds_several() {
    let home = folder();
    let db_path = home.join(".local/share/opencode/opencode.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let db = rusqlite::Connection::open(&db_path).unwrap();
    db.execute_batch("CREATE TABLE credential (id TEXT, integration_id TEXT, value TEXT);").unwrap();
    db.execute("INSERT INTO credential VALUES ('c1', 'provider-a', ?1)", [r#"{"type":"key","key":"fake-a-5678"}"#]).unwrap();
    assert_eq!(identity_of(NAME, &looking(&home, NOTHING, NOTHING)), AccountIdentity::of_key("provider-a", "fake-a-5678"));
    db.execute("INSERT INTO credential VALUES ('c2', 'provider-b', ?1)", [r#"{"type":"key","key":"fake-b-9999"}"#]).unwrap();
    assert_eq!(identity_of(NAME, &looking(&home, NOTHING, NOTHING)), AccountIdentity::None, "two keys and nothing says which is in use");
    let _ = std::fs::remove_dir_all(home);
}
