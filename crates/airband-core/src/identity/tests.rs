use super::*;
use serde_json::json;

fn row(tool: &str) -> std::collections::BTreeMap<String, Value> {
    let t = crate::catalog::built().tools.iter().find(|t| t.id == tool).unwrap_or_else(|| panic!("no {tool} row"));
    t.with.clone()
}

#[test]
fn a_key_is_known_by_its_fingerprint_and_shown_by_its_last_four() {
    let a = AccountIdentity::of_key("fixture-provider", "sk-abcdef1234");
    let AccountIdentity::Key { fingerprint, last4 } = &a else { panic!() };
    assert_eq!(last4, "1234");
    assert_eq!(fingerprint.len(), 64);
    assert!(!fingerprint.contains("abcdef"));
    // the same key on another computer is the same account; another key ending alike is not
    assert!(a.same(&AccountIdentity::of_key("fixture-provider", "sk-abcdef1234")));
    assert!(!a.same(&AccountIdentity::of_key("fixture-provider", "sk-zzzzzz1234")));
    assert!(!a.same(&AccountIdentity::of_key("fixture-other", "sk-abcdef1234")));
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
    assert!(serde_json::from_value::<AccountIdentity>(json!({"kind": "something_new"})).is_err(), "a kind it does not have is refused");
    assert_eq!(serde_json::from_value::<AccountIdentity>(serde_json::to_value(&one).unwrap()).unwrap(), one);
}

#[test]
fn every_tool_row_says_where_its_account_is_and_each_source_reads() {
    // what each tool's sources give is held to in that tool's own folder (`adapters/tools/<tool>/row.rs`)
    for tool in &crate::catalog::built().tools {
        assert!(!sources(&row(&tool.id)).unwrap().is_empty(), "{} names no account source", tool.id);
    }
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
