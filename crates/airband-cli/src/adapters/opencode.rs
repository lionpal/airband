//! OpenCode, in a terminal he opened himself or its own window.
//!
//! OpenCode has no hooks. It runs one small server on the computer that every one of its sessions goes
//! through, and says where it is in a file of its own (`service.json`: an address on this computer and a
//! password). So nothing is installed and nothing has to be trusted: the tower looks at that server every
//! couple of seconds and tells itself what it sees, as the same events any other tool sends.
//!
//! - A session that became busy is working. One that became idle has spoken: its newest answer is what it said.
//! - His words go in through the server (`POST …/prompt`), which the session takes as a turn of his own.
//!
//! Not here yet: its requests for leave. They can be listed and answered through the same server, and a
//! session only asks when he has set it to.
use airband_core::agent::{Agent, AgentEvent, Deliver, Event, SCHEMA};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

const NAME: &str = "opencode";
/// A session that has not changed for this long is not on the radio.
const QUIET: u64 = 2 * 3600 * 1000;

/// Where OpenCode's server is and the password for it, when it is running.
fn service() -> Option<(String, String)> {
    let state = std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| crate::config::home().join(".local").join("state"));
    let v: Value = serde_json::from_slice(&std::fs::read(state.join("opencode").join("service.json")).ok()?).ok()?;
    Some((v["url"].as_str()?.trim_end_matches('/').to_string(), v["password"].as_str()?.to_string()))
}

fn client() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(4))).http_status_as_error(false).build().into()
}
fn auth(password: &str) -> String {
    format!("Basic {}", airband_core::audio::base64_encode(format!("opencode:{password}").as_bytes()))
}
fn get(path: &str) -> Option<Value> {
    let (url, password) = service()?;
    let mut res = client().get(format!("{url}{path}")).header("authorization", auth(&password)).call().ok()?;
    (res.status().as_u16() == 200).then(|| res.body_mut().read_json::<Value>().ok()).flatten()
}

fn agent(id: &str, cwd: Option<&str>) -> Agent {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "airband".into());
    Agent {
        id: id.into(),
        harness: NAME.into(),
        cwd: cwd.map(String::from),
        name: None,
        title: None,
        pid: None,
        // his words are handed to this program, which passes them to OpenCode's server
        deliver: Some(Deliver::Command { argv: vec![exe, "deliver".into(), NAME.into(), id.into()] }),
        meta: Default::default(),
    }
}

/// What a session's newest answer says: its text, without what it thought to itself on the way.
fn newest_answer(messages: &Value) -> Option<String> {
    let said = messages["data"].as_array()?.iter().find(|m| m["type"] == "assistant")?;
    let text: Vec<&str> = said["content"].as_array()?.iter().filter(|part| part["type"] == "text").filter_map(|part| part["text"].as_str()).map(str::trim).filter(|t| !t.is_empty()).collect();
    (!text.is_empty()).then(|| text.join("\n\n"))
}

/// Watch OpenCode's server for as long as the tower runs. Blocks: run it on a thread of its own.
pub fn watch() {
    // each session seen: when it was last busy and last idle, by OpenCode's own clock
    let mut seen: HashMap<String, (u64, u64)> = HashMap::new();
    let mut first = true;
    loop {
        std::thread::sleep(Duration::from_secs(if service().is_some() { 2 } else { 20 }));
        let Some(list) = get("/api/session?limit=40") else { continue };
        let Some(sessions) = list["data"].as_array() else { continue };
        let now = sessions.iter().filter_map(|s| s["time"]["updated"].as_u64()).max().unwrap_or(0);
        let send = |id: &str, cwd: Option<&str>, event: Event| super::post(&AgentEvent { schema: SCHEMA.into(), agent: agent(id, cwd), event });
        for s in sessions {
            let (Some(id), time) = (s["id"].as_str(), &s["time"]) else { continue };
            let (busy, idle, updated) = (time["busy"].as_u64().unwrap_or(0), time["idle"].as_u64().unwrap_or(0), time["updated"].as_u64().unwrap_or(0));
            let cwd = s["location"]["directory"].as_str();
            match seen.get(id).copied() {
                None => {
                    // what was already there when the tower started has been said already: it is on the radio
                    // if it is recent, and nothing of it is said again
                    if now.saturating_sub(updated) > QUIET {
                        continue;
                    }
                    seen.insert(id.to_string(), (busy, idle));
                    send(id, cwd, Event::Checkin);
                    if !first && busy > idle {
                        send(id, cwd, Event::Working);
                    }
                }
                Some((was_busy, was_idle)) => {
                    if busy > was_busy && busy > idle {
                        send(id, cwd, Event::Working);
                    }
                    if idle > was_idle && idle >= busy {
                        if let Some(text) = get(&format!("/api/session/{id}/message?limit=6")).as_ref().and_then(newest_answer) {
                            send(id, cwd, Event::Said { text, kind: None });
                        }
                    }
                    seen.insert(id.to_string(), (busy.max(was_busy), idle.max(was_idle)));
                }
            }
        }
        // one that has gone from the list (he archived or deleted it), or has been quiet for hours, has left
        let here: Vec<&str> = sessions.iter().filter(|s| now.saturating_sub(s["time"]["updated"].as_u64().unwrap_or(0)) <= QUIET).filter_map(|s| s["id"].as_str()).collect();
        let gone: Vec<String> = seen.keys().filter(|id| !here.contains(&id.as_str())).cloned().collect();
        for id in gone {
            seen.remove(&id);
            send(&id, None, Event::Signoff);
        }
        first = false;
    }
}

/// `airband deliver opencode <session>`: his words, on standard input, into that session as a turn of his own.
pub fn deliver(session: &str) -> Result<(), String> {
    let mut words = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut words).map_err(|e| e.to_string())?;
    let (url, password) = service().ok_or("OpenCode is not running on this computer.")?;
    let res = client().post(format!("{url}/api/session/{session}/prompt")).header("authorization", auth(&password)).send_json(serde_json::json!({ "text": words.trim() })).map_err(|e| format!("OpenCode could not be reached: {e}"))?;
    match res.status().as_u16() {
        200..=299 => Ok(()),
        code => Err(format!("OpenCode did not take it ({code}).")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_answer_is_its_words_without_its_thinking() {
        let messages = json!({ "data": [
            { "type": "idle", "outcome": "succeeded" },
            { "type": "assistant", "content": [{ "type": "reasoning", "text": "He wants one word." }, { "type": "text", "text": " queued " }] },
            { "type": "user", "text": "Now reply with the one word: queued" },
            { "type": "assistant", "content": [{ "type": "text", "text": "ready" }] },
        ] });
        assert_eq!(newest_answer(&messages).as_deref(), Some("queued"));
        assert_eq!(newest_answer(&json!({ "data": [{ "type": "user", "text": "hello" }] })), None);
        // his words go to the session through this program
        match agent("ses_1", Some("/x/shop")).deliver {
            Some(Deliver::Command { argv }) => assert_eq!(&argv[1..], ["deliver", "opencode", "ses_1"]),
            other => panic!("{other:?}"),
        }
    }
}
