use super::*;
use serde_json::json;

fn row(tool: &str) -> std::collections::BTreeMap<String, Value> {
    let t = crate::catalog::built().tools.iter().find(|t| t.id == tool).unwrap_or_else(|| panic!("no {tool} row"));
    t.with.clone()
}

#[test]
fn a_key_is_known_by_its_fingerprint_and_shown_by_its_last_four() {
    let a = AccountIdentity::of_key("openai", "sk-abcdef1234");
    let AccountIdentity::Key { fingerprint, last4 } = &a else { panic!() };
    assert_eq!(last4, "1234");
    assert_eq!(fingerprint.len(), 64);
    assert!(!fingerprint.contains("abcdef"));
    // the same key on another computer is the same account; another key ending alike is not
    assert!(a.same(&AccountIdentity::of_key("openai", "sk-abcdef1234")));
    assert!(!a.same(&AccountIdentity::of_key("openai", "sk-zzzzzz1234")));
    assert!(!a.same(&AccountIdentity::of_key("anthropic", "sk-abcdef1234")));
    assert_eq!(super::last4("ab"), "", "a short key shows nothing of itself");
    assert_eq!(super::last4("sk-abcdef1234"), "1234");
}

#[test]
fn an_unknown_account_is_never_taken_for_another() {
    assert!(!AccountIdentity::None.same(&AccountIdentity::None));
    let one = AccountIdentity::Id { id: "p".into(), org: Some("w1".into()) };
    assert!(one.same(&one.clone()));
    // one person in two workspaces: two allowances
    assert!(!one.same(&AccountIdentity::Id { id: "p".into(), org: Some("w2".into()) }));
    assert_eq!(serde_json::from_value::<AccountIdentity>(json!({"kind": "something_new"})).unwrap(), AccountIdentity::None);
    assert_eq!(serde_json::from_value::<AccountIdentity>(serde_json::to_value(&one).unwrap()).unwrap(), one);
}

#[test]
fn every_tool_row_says_where_its_account_is_and_each_source_reads() {
    for tool in ["claude-code", "codex", "opencode"] {
        assert!(!sources(&row(tool)).unwrap().is_empty(), "{tool} names no account source");
    }
}

#[test]
fn claude_codes_account_is_its_account_id_in_its_organisation() {
    let s = sources(&row("claude-code")).unwrap();
    let doc = json!({"oauthAccount": {"accountUuid": "acc-1", "organizationUuid": "org-1", "emailAddress": "x"}});
    assert_eq!(s[0].identity(&doc, None), Some(AccountIdentity::Id { id: "acc-1".into(), org: Some("org-1".into()) }));
    assert_eq!(s[0].identity(&json!({}), None), None);
    assert!(matches!(s[1].identity(&json!("sk-ant-fake-key-xyz9"), None), Some(AccountIdentity::Key { last4, .. }) if last4 == "xyz9"));
}

#[test]
fn codexs_account_is_the_person_in_the_workspace_from_its_token() {
    use base64::Engine;
    let claims = json!({"https://api.openai.com/auth": {"chatgpt_user_id": "user-1", "chatgpt_account_id": "ws-1"}});
    let body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
    let doc = json!({"auth_mode": "chatgpt", "tokens": {"id_token": format!("h.{body}.s")}, "OPENAI_API_KEY": null});
    let s = sources(&row("codex")).unwrap();
    assert_eq!(s[0].identity(&doc, None), Some(AccountIdentity::Id { id: "user-1".into(), org: Some("ws-1".into()) }));
    // signed in with a key: no token, the key is the account
    let keyed = json!({"auth_mode": "apikey", "OPENAI_API_KEY": "sk-proj-0000abcd"});
    assert_eq!(s[0].identity(&keyed, None), None);
    assert_eq!(s[1].identity(&keyed, None), Some(AccountIdentity::of_key("openai", "sk-proj-0000abcd")));
}

#[test]
fn opencodes_keys_are_known_by_the_provider_each_row_names() {
    let s = sources(&row("opencode")).unwrap();
    assert_eq!(s[0].identity(&json!({"type": "key", "key": "k-1234"}), Some("opencode-go")), Some(AccountIdentity::of_key("opencode-go", "k-1234")));
    // a sign-in that is not a key records no account here
    assert_eq!(s[0].identity(&json!({"type": "oauth", "refresh": "r"}), Some("p")), None);
}

#[test]
fn a_source_that_can_never_give_anything_is_refused() {
    let bad = |v: Value| sources(&[("account_id".to_string(), v)].into_iter().collect()).is_err();
    assert!(bad(json!([{"place": {"kind": "env", "name": "X"}, "take": {"kind": "id", "path": ["a"]}}])));
    assert!(bad(json!([{"place": {"kind": "env", "name": "X"}, "take": {"kind": "key"}}])));
    assert!(bad(json!([{"place": {"kind": "file", "file": "a", "under": "home"}, "take": {"kind": "id", "path": []}}])));
    assert!(bad(json!([{"place": {"kind": "moon"}, "take": {"kind": "key", "provider": "p"}}])));
    assert!(sources(&Default::default()).unwrap().is_empty());
}
