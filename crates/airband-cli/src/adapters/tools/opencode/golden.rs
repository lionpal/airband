//! What this adapter makes, as it made it when written down, held to (`adapters/golden.rs`).
//! OpenCode has no hooks: everything else it says comes from its own server, which is not running here.
use super::NAME;
use crate::adapters::golden::*;
use crate::adapters::{agent_with, harness, put_away};
use serde_json::json;

#[test]
fn what_opencode_reports_is_what_it_reported_before() {
    let s = Scratch::new("opencode");
    let steps = s.steps(include_str!("fixtures/hooks.json"));
    let reported = reported(&s, NAME, &steps);
    let adapter = harness(NAME).unwrap();
    let agent = agent_with("ses_1", NAME);
    let request = json!({ "id": "per_1", "sessionID": "ses_1", "action": "shell", "resources": ["echo airband-probe"], "save": ["echo *"] });
    let made = json!({
        "reported": reported,
        "installed": hooks_put(NAME),
        "looked": { "glance": seen(&adapter.glance(&agent)), "touched": adapter.touched(&agent), "steps": adapter.steps(&agent, 0), "told": adapter.told(&agent, 0).1 },
        "worked": folders(adapter.worked()),
        "accounts": adapter.accounts().is_some(),
        "puts_away": adapter.puts_away(),
        "put_away": put_away(NAME, "ses_1"),
        "deliver": adapter.deliver("ses_1", "hello"),
        "open": adapter.open().map(|o| o.open.len()),
        "leave": [super::leave(&request, Some("/x/shop")), super::leave(&json!({ "id": "per_2", "sessionID": "ses_1", "action": "edit", "resources": ["src/a.rs", "src/b.rs"] }), None), super::leave(&json!({ "id": "per_3", "sessionID": "ses_1", "action": "webfetch", "resources": [] }), None)],
    });
    s.hold(NAME, &made, include_str!("fixtures/golden.json"));
}
