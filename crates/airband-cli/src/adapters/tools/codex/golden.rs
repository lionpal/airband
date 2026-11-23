//! What this adapter makes of Codex's real-shaped notifications and records, as it made it when written down, held to
//! (`adapters/golden.rs`): a change to code it shares with other tools changes nothing it reports.
use super::NAME;
use crate::adapters::golden::*;
use crate::adapters::{agent_with, harness, put_away};
use serde_json::json;

#[test]
fn what_codex_reports_is_what_it_reported_before() {
    let s = Scratch::new("codex");
    let steps = s.steps(include_str!("fixtures/hooks.json"));
    let reported = reported(&s, NAME, &steps);
    let adapter = harness(NAME).unwrap();
    // its hooks are put in place only once Codex is on this computer
    let before = hooks_put(NAME);
    std::fs::create_dir_all(s.path("home/.codex")).unwrap();
    let installed = hooks_put(NAME);
    let agent = airband_core::agent::Agent { meta: serde_json::Map::from_iter([("model".to_string(), json!("gpt-6-luna"))]), ..agent_with("t1", NAME) };
    let looked = json!({ "glance": seen(&adapter.glance(&agent)), "bare": seen(&adapter.glance(&agent_with("t2", NAME))), "touched": adapter.touched(&agent), "steps": adapter.steps(&agent, 0), "told": adapter.told(&agent, 0).1 });
    let meta = |cwd: &str| format!("{}\n{}\n", json!({ "type": "session_meta", "payload": { "id": "t", "cwd": cwd } }), json!({ "type": "event", "payload": { "cwd": "/not/this" } }));
    let day = "home/.codex/sessions/2026/11/21";
    for (file, text, at) in [
        (format!("{day}/rollout-a.jsonl"), meta("/x/shop"), 1_000),
        (format!("{day}/rollout-b.jsonl"), meta("/x/shop"), 2_000),
        (format!("{day}/rollout-c.jsonl"), meta("/x/garden"), 1_500),
        (format!("{day}/rollout-d.jsonl"), String::new(), 3_000),
        (format!("{day}/rollout-e.jsonl"), format!("{}\n", json!({ "type": "event", "payload": { "cwd": "/x/notes" } })), 3_000),
        (format!("{day}/rollout-f.jsonl"), format!("{}\n{}\n", json!({ "type": "event" }), json!({ "type": "session_meta", "payload": { "cwd": "/x/second-line" } })), 3_000),
        ("home/.codex/sessions/2026/12/31/rollout-z.jsonl".to_string(), meta("/x/late"), 100),
        ("home/.codex/sessions/2026/12/loose.jsonl".to_string(), meta("/x/loose"), 4_000),
    ] {
        s.step(&json!({ "file": { "path": file, "text": text, "written": at } }));
    }
    let made = json!({
        "reported": reported,
        "installed_before_it_is_here": before,
        "installed": installed,
        "looked": looked,
        "worked": folders(adapter.worked()),
        "accounts": adapter.accounts().is_some(),
        "puts_away": adapter.puts_away(),
        "put_away": put_away(NAME, "t1"),
        "open": adapter.open().map(|o| o.len()),
    });
    s.hold(NAME, &made, include_str!("fixtures/golden.json"));
}
