//! What this adapter makes of Antigravity CLI's real-shaped hooks and records, as it made it when written
//! down, held to (`adapters/golden.rs`): a change to code it shares with other tools changes nothing it reports.
use super::NAME;
use crate::adapters::golden::*;
use crate::adapters::{agent_with, harness, put_away};
use serde_json::json;

#[test]
fn what_antigravity_cli_reports_is_what_it_reported_before() {
    let s = Scratch::new("antigravity-cli");
    let steps = s.steps(include_str!("fixtures/hooks.json"));
    let reported = reported(&s, NAME, &steps);
    let adapter = harness(NAME).unwrap();
    // its hooks are put in place only once it has been run on this computer: its own folder is there. The brain
    // written by the steps is under it, so the look before is with that folder taken away
    let state = s.path("home/.gemini/antigravity-cli");
    let aside = s.path("aside");
    std::fs::rename(&state, &aside).unwrap();
    let before = hooks_put(NAME);
    std::fs::rename(&aside, &state).unwrap();
    // a hooks file with his own entry in it already
    s.step(&json!({ "file": { "path": "home/.gemini/config/hooks.json", "json": { "lint": { "PostToolUse": [{ "matcher": "run_command", "hooks": [{ "command": "./lint.sh" }] }] } } } }));
    let installed = hooks_put(NAME);
    let agent = airband_core::agent::Agent { meta: serde_json::Map::from_iter([("model".to_string(), json!("gemini-4-neon-low"))]), ..agent_with("c1", NAME) };
    let looked = json!({ "glance": seen(&adapter.glance(&agent)), "touched": adapter.touched(&agent), "steps": adapter.steps(&agent, 0), "told": adapter.told(&agent, 0).1 });
    for (file, text, at) in [
        ("home/.gemini/antigravity-cli/cache/last_conversations.json", json!({ "/x/shop": "c1", "/x/garden": "c2", "/x/gone": "c3" }).to_string(), 5_000),
        ("home/.gemini/antigravity-cli/conversations/c1.db", String::new(), 2_000),
        ("home/.gemini/antigravity-cli/conversations/c2.db", String::new(), 1_000),
    ] {
        s.step(&json!({ "file": { "path": file, "text": text, "written": at } }));
    }
    // one conversation open (its lock held), one closed (its lock let go)
    let presence = s.path("home/.gemini/antigravity-cli/presence");
    let held = airband_io::files::lock(&presence.join("c1.lock")).unwrap();
    drop(airband_io::files::lock(&presence.join("c2.lock")).unwrap());
    let open: Vec<serde_json::Value> = adapter.open().map(|l| l.open).unwrap_or_default().iter().map(|o| json!({ "agent": o.agent, "idle": o.idle })).collect();
    drop(held);
    let made = json!({
        "reported": reported,
        "installed_before_it_is_here": before,
        "installed": installed,
        "looked": looked,
        "worked": folders(adapter.worked()),
        "accounts": adapter.accounts().is_some(),
        "puts_away": adapter.puts_away(),
        "put_away": put_away(NAME, "c1"),
        "deliver": adapter.deliver("c1", "hello"),
        "open": open,
        "open_when_closed": adapter.open().map(|l| l.open.len()),
    });
    s.hold(NAME, &made, include_str!("fixtures/golden.json"));
}
