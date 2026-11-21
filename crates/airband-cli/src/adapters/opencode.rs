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
//! - Its requests for leave are listed by the same server (`GET …/permission`) and answered there
//!   (`POST …/permission/<id>/reply`). Each is put to him, and his answer is the one that is sent: once, or
//!   refused with his reason. One nobody answered from a phone stays where it was, on OpenCode's own screen.
use super::Tower;
use airband_core::agent::{Agent, AgentEvent, Deliver, Event, PermissionAsk, SCHEMA};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

fn post(path: &str, body: Value) -> Option<u16> {
    let (url, password) = service()?;
    client().post(format!("{url}{path}")).header("authorization", auth(&password)).send_json(body).ok().map(|res| res.status().as_u16())
}

fn agent(id: &str, cwd: Option<&str>) -> Agent {
    Agent {
        id: id.into(),
        harness: NAME.into(),
        cwd: cwd.map(String::from),
        name: None,
        title: None,
        pid: None,
        // his words go to the session through OpenCode's server (`deliver` below)
        deliver: Some(Deliver::Harness { name: NAME.into(), session: id.into() }),
        meta: Default::default(),
    }
}

/// What a session's newest answer says: its text, without what it thought to itself on the way.
fn newest_answer(messages: &Value) -> Option<String> {
    let said = messages["data"].as_array()?.iter().find(|m| m["type"] == "assistant")?;
    let text: Vec<&str> = said["content"].as_array()?.iter().filter(|part| part["type"] == "text").filter_map(|part| part["text"].as_str()).map(str::trim).filter(|t| !t.is_empty()).collect();
    (!text.is_empty()).then(|| text.join("\n\n"))
}

/// A request for leave of OpenCode's, as he is asked it. `action` is what it wants to do in OpenCode's word
/// ("shell", "edit") and `resources` what to: the command, the files. What he reads is those, whole.
fn leave(request: &Value, cwd: Option<&str>) -> Option<PermissionAsk> {
    let (session, action) = (request["sessionID"].as_str()?, request["action"].as_str()?);
    let on: Vec<&str> = request["resources"].as_array()?.iter().filter_map(Value::as_str).collect();
    let what = match (action, on.as_slice()) {
        ("shell", [command]) => format!("run `{command}`"),
        (_, []) => format!("use {action}"),
        _ => format!("{action}: {}", on.join(", ")),
    };
    // one too long to put in front of him whole is not asked on the phone: it stays on OpenCode's own screen
    super::hooks::leave(agent(session, cwd), action, what, request, " The rest is on OpenCode's own screen.", None)
}

/// Requests for leave that are being put to him, or were: each is asked once.
#[derive(Default)]
struct Asked {
    done: HashSet<String>,
    /// One that found nobody there to ask is tried again, no sooner than this.
    again: HashMap<String, Instant>,
}

/// Put one of a session's requests to him and send his answer back. Runs for as long as he takes.
fn put_to_him(session: String, request: Value, cwd: Option<String>, asked: Arc<Mutex<Asked>>) {
    let Some(id) = request["id"].as_str().map(String::from) else { return };
    let Some(ask) = leave(&request, cwd.as_deref()) else {
        asked.lock().unwrap().done.insert(id);
        return;
    };
    // it can be answered on OpenCode's own screen meanwhile: then it is no longer in the list, and the question
    // is taken back from the phone
    let still = || get(&format!("/api/session/{session}/permission")).is_none_or(|list| list["data"].as_array().is_some_and(|l| l.iter().any(|r| r["id"] == id.as_str())));
    match super::ask_him_while(&ask, Some(&still)) {
        super::Answer::Nobody => {
            asked.lock().unwrap().again.insert(id, Instant::now() + Duration::from_secs(20));
        }
        answer => {
            if let super::Answer::Said(allow, reason) = answer {
                let message = if allow { Value::Null } else if reason.is_empty() { "He said no from his phone.".into() } else { format!("He said no from his phone: {reason}").into() };
                let _ = post(&format!("/api/session/{session}/permission/{id}/reply"), json!({ "decision": if allow { "once" } else { "reject" }, "message": message }));
            }
            let mut asked = asked.lock().unwrap();
            asked.again.remove(&id);
            asked.done.insert(id);
        }
    }
}

/// Watch OpenCode's server for as long as the tower runs. Blocks: run it on a thread of its own.
fn watch(tower: Tower) {
    // each session seen: when it was last busy and last idle, by OpenCode's own clock, as far as the tower has
    // been told. Something the tower did not take is not marked, and is told again on the next look.
    let mut seen: HashMap<String, (u64, u64)> = HashMap::new();
    let asked: Arc<Mutex<Asked>> = Default::default();
    let mut first = true;
    loop {
        std::thread::sleep(Duration::from_secs(if service().is_some() { 2 } else { 20 }));
        let Some(list) = get("/api/session?limit=40") else { continue };
        let Some(sessions) = list["data"].as_array() else { continue };
        let now = sessions.iter().filter_map(|s| s["time"]["updated"].as_u64()).max().unwrap_or(0);
        let send = |id: &str, cwd: Option<&str>, event: Event| (tower.event)(AgentEvent { schema: SCHEMA.into(), agent: agent(id, cwd), event });
        // the ones on the radio: not the ones the tower runs itself (their driver reports for them), nor the
        // ones put away (a one-off run, a summary being made), nor the ones quiet for hours
        let ours = |s: &&Value| s["id"].as_str().is_some_and(|id| !(tower.holds)(id)) && !(s["time"]["archived"].as_u64().is_some_and(|at| at > 0) || s["time"]["archived"] == true);
        for s in sessions.iter().filter(ours) {
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
                    if send(id, cwd, Event::Checkin) {
                        let working = !first && busy > idle && send(id, cwd, Event::Working);
                        seen.insert(id.to_string(), (if first || working { busy } else { 0 }, idle));
                    }
                }
                Some((was_busy, was_idle)) => {
                    let (mut told_busy, mut told_idle) = (was_busy, was_idle);
                    if busy > was_busy && busy > idle && send(id, cwd, Event::Working) {
                        told_busy = busy;
                    }
                    if idle > was_idle && idle >= busy {
                        // its turn is over: its newest answer is what it said, and with none it only stopped
                        let ended = match get(&format!("/api/session/{id}/message?limit=6")) {
                            Some(messages) => Some(newest_answer(&messages).map(|text| Event::Said { text, kind: None }).unwrap_or(Event::Stopped { why: None })),
                            None => None,
                        };
                        if ended.is_some_and(|event| send(id, cwd, event)) {
                            (told_busy, told_idle) = (told_busy.max(busy), idle);
                        }
                    }
                    seen.insert(id.to_string(), (told_busy, told_idle));
                }
            }
            // while it is busy it may be waiting for leave
            if busy > idle && seen.contains_key(id) {
                for request in get(&format!("/api/session/{id}/permission")).and_then(|l| l["data"].as_array().cloned()).unwrap_or_default() {
                    let Some(request_id) = request["id"].as_str() else { continue };
                    {
                        let mut asked = asked.lock().unwrap();
                        if asked.done.contains(request_id) || asked.again.get(request_id).is_some_and(|at| Instant::now() < *at) {
                            continue;
                        }
                        // not again until this asking is over
                        asked.again.insert(request_id.to_string(), Instant::now() + Duration::from_secs(3600));
                    }
                    let (session, cwd, asked) = (id.to_string(), cwd.map(String::from), asked.clone());
                    std::thread::spawn(move || put_to_him(session, request, cwd, asked));
                }
            }
        }
        // one that has gone from the list (he archived or deleted it), or has been quiet for hours, has left
        let here: Vec<&str> = sessions.iter().filter(ours).filter(|s| now.saturating_sub(s["time"]["updated"].as_u64().unwrap_or(0)) <= QUIET).filter_map(|s| s["id"].as_str()).collect();
        let gone: Vec<String> = seen.keys().filter(|id| !here.contains(&id.as_str())).cloned().collect();
        for id in gone {
            if send(&id, None, Event::Signoff) {
                seen.remove(&id);
            }
        }
        first = false;
    }
}

/// His words into that session as a turn of his own.
fn deliver(session: &str, words: &str) -> Result<(), String> {
    service().ok_or("OpenCode is not running on this computer.")?;
    match post(&format!("/api/session/{session}/prompt"), json!({ "text": words })) {
        Some(200..=299) => Ok(()),
        Some(code) => Err(format!("OpenCode did not take it ({code}).")),
        None => Err("OpenCode could not be reached.".into()),
    }
}

pub struct OpenCode;

impl super::Harness for OpenCode {
    fn name(&self) -> &'static str {
        NAME
    }
    fn watch(&self, tower: Tower) {
        watch(tower)
    }
    fn deliver(&self, session: &str, words: &str) -> Result<(), String> {
        deliver(session, words)
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
        // his words go to the session through OpenCode's server
        assert_eq!(agent("ses_1", Some("/x/shop")).deliver, Some(Deliver::Harness { name: "opencode".into(), session: "ses_1".into() }));
    }

    #[test]
    fn a_request_for_leave_is_put_as_what_will_run() {
        // as OpenCode 2.3.2 lists one
        let request = json!({ "id": "per_1", "sessionID": "ses_1", "action": "shell", "resources": ["echo airband-probe"], "save": ["echo *"], "source": { "type": "tool", "messageID": "msg_1", "id": "call_1" } });
        let ask = leave(&request, Some("/x/shop")).unwrap();
        assert_eq!((ask.what.as_str(), ask.agent.id.as_str()), ("run `echo airband-probe`", "ses_1"));
        assert!(ask.detail.starts_with("shell\n") && ask.detail.contains("echo airband-probe"));
        assert_eq!(leave(&json!({ "id": "per_2", "sessionID": "ses_1", "action": "edit", "resources": ["src/a.rs", "src/b.rs"] }), None).unwrap().what, "edit: src/a.rs, src/b.rs");
    }
}
