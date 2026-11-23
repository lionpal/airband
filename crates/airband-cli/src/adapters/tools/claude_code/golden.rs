//! What this adapter makes of Claude Code's real-shaped notifications, transcript and records, as it made it when written down, held to
//! (`adapters/golden.rs`): a change to code it shares with other tools changes nothing it reports.
use super::HARNESS;
use crate::adapters::golden::*;
use crate::adapters::{agent_with, harness, put_away};
use serde_json::{Value, json};

fn set_written(path: &std::path::Path, secs: u64) {
    std::fs::File::options().write(true).open(path).unwrap().set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)).unwrap();
}

#[test]
fn what_claude_code_reports_is_what_it_reported_before() {
    let s = Scratch::new("claude-code");
    let steps = s.steps(include_str!("fixtures/hooks.json"));
    let reported = reported(&s, HARNESS, &steps);
    // his own hook is in the file already, and stays
    s.step(&json!({ "file": { "path": "home/.claude/settings.json", "json": { "model": "x", "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "afplay ding.aiff" }] }] } } } }));
    let installed = hooks_put(HARNESS);
    let adapter = harness(HARNESS).unwrap();
    let transcript = s.path("home/.claude/projects/-x-shop/s1.jsonl");
    let agent = airband_core::agent::Agent { meta: serde_json::Map::from_iter([("transcript".to_string(), Value::String(transcript.display().to_string()))]), ..agent_with("s1", HARNESS) };
    let (told, at) = adapter.told(&agent, 0);
    let (again, at_again) = adapter.told(&agent, at);
    let told: Vec<Value> = told.iter().map(|t| json!({ "id": t.id, "at": t.at, "text": t.text, "images": t.images, "his": t.his })).collect();
    let looked = json!({
        "glance": seen(&adapter.glance(&agent)),
        "touched": adapter.touched(&agent),
        "steps": adapter.steps(&agent, 0),
        "steps_since": adapter.steps(&agent, 1_795_109_940_000),
        "told": told, "at": at, "told_again": again.len(), "at_again": at_again,
        "no_transcript": seen(&adapter.glance(&agent_with("s2", HARNESS))),
    });
    // where it has worked: the newest transcript of each project that says where it was
    let line = |cwd: &str| json!({ "type": "user", "cwd": cwd }).to_string();
    for (file, text, at) in [
        ("home/.claude/projects/-x-garden/c.jsonl", format!("{}\n{}\n", json!({ "type": "queue-operation" }), line("/x/garden")), 1_000),
        ("home/.claude/projects/-x-shop/old.jsonl", format!("{}\n", line("/x/old-shop")), 500),
        ("home/.claude/projects/-x-barn/new.jsonl", format!("{}\n", json!({ "type": "summary" })), 3_000),
        ("home/.claude/projects/-x-barn/old.jsonl", format!("{}\n", line("/x/barn")), 1_500),
        ("home/.claude/projects/-x-far/a.jsonl", format!("{}{}\n", format!("{}\n", json!({ "type": "x" })).repeat(40), line("/x/far")), 1_200),
    ] {
        s.step(&json!({ "file": { "path": file, "text": text } }));
        set_written(&s.path(file), at);
    }
    std::fs::create_dir_all(s.path("home/.claude/projects/-x-empty")).unwrap();
    set_written(&transcript, 2_000);
    let made = json!({
        "reported": reported,
        "installed": installed,
        "looked": looked,
        "worked": folders(adapter.worked()),
        "accounts": adapter.accounts().is_some(),
        "puts_away": adapter.puts_away(),
        "deliver": adapter.deliver("s1", "hello"),
        "put_away": put_away(HARNESS, "s1"),
    });
    s.hold(HARNESS, &made, include_str!("fixtures/golden.json"));
}
