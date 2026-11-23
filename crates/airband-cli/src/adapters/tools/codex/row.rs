//! What Codex's row says, as this code asks it, and its account read the way the row says to.
use super::NAME;
use crate::adapters::accounts::tests::{NOTHING, folder, looking};
use crate::adapters::{identity_of, row_says, tool_row};
use airband_core::identity::AccountIdentity;
use serde_json::json;

#[test]
fn every_row_fact_this_code_asks_for_is_there_and_his_words_are_one_whole_word_of_its_command() {
    for fact in ["home", "home_env", "program", "control_socket", "hooks_file", "sessions"] {
        assert!(!row_says(NAME, fact).is_empty(), "its row says nothing for {fact}");
    }
    // it is handed words by the command its row gives, his words one whole word of it
    let run = tool_row(NAME).unwrap().command("deliver", &[("program", "prog"), ("session", "t1"), ("words", "--yes {session}")]);
    assert_eq!(run.first().map(String::as_str), Some("prog"));
    assert!(run.contains(&"t1".to_string()) && run.contains(&"--yes {session}".to_string()) && !run.iter().any(|w| w == "{words}"), "{run:?}");
}

#[test]
fn its_account_is_the_person_in_the_workspace_from_its_token() {
    let claims = json!({"https://api.openai.com/auth": {"chatgpt_user_id": "user-1", "chatgpt_account_id": "ws-1"}});
    let body = airband_core::encode::base64_url(&serde_json::to_vec(&claims).unwrap());
    let doc = json!({"auth_mode": "chatgpt", "tokens": {"id_token": format!("h.{body}.s")}, "OPENAI_API_KEY": null});
    let s = airband_core::identity::sources(&tool_row(NAME).unwrap().with).unwrap();
    assert_eq!(s[0].identity(&doc, None), Some(AccountIdentity::Id { id: "user-1".into(), org: Some("ws-1".into()) }));
    // signed in with a key: no token, the key is the account
    let keyed = json!({"auth_mode": "apikey", "OPENAI_API_KEY": "sk-proj-0000abcd"});
    assert_eq!(s[0].identity(&keyed, None), None);
    assert_eq!(s[1].identity(&keyed, None), Some(AccountIdentity::of_key("openai", "sk-proj-0000abcd")));
}

#[test]
fn an_id_inside_a_token_is_read_from_its_payload_and_the_person_is_told_from_the_workspace() {
    let home = folder();
    let auth = json!({ "https://api.openai.com/auth": { "chatgpt_user_id": "person-1", "chatgpt_account_id": "workspace-1" } });
    let token = format!("head.{}.sig", airband_core::encode::base64_url(auth.to_string().as_bytes()));
    std::fs::write(home.join("auth.json"), json!({ "tokens": { "id_token": token } }).to_string()).unwrap();
    assert_eq!(identity_of(NAME, &looking(&home, NOTHING, NOTHING)), AccountIdentity::Id { id: "person-1".into(), org: Some("workspace-1".into()) });
    // signed in with a key instead: the key in the same file
    std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"fake-openai-1234"}"#).unwrap();
    assert_eq!(identity_of(NAME, &looking(&home, NOTHING, NOTHING)), AccountIdentity::of_key("openai", "fake-openai-1234"));
    let _ = std::fs::remove_dir_all(home);
}
