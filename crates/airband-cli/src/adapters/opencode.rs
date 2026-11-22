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
/// What the tool is called, as its row has it.
fn tool() -> String {
    super::called(NAME).0
}
/// A session that has not changed for this long is not on the radio. OpenCode's server keeps every session
/// there has been and does not say which have a window open (it runs a session's turns itself, window or no
/// window), so there is no fact to go by but this and whether one was put away.
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
    super::hooks::leave(agent(session, cwd), action, what, request, &format!(" The rest is on the screen of {} itself.", tool()), None)
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

/// The sessions on the radio, of those the server lists: not the ones put away (a one-off run, a summary
/// being made), nor the ones quiet for hours.
fn current(sessions: &[Value], now: u64) -> Vec<&Value> {
    let put_away = |s: &Value| s["time"]["archived"].as_u64().is_some_and(|at| at > 0) || s["time"]["archived"] == true;
    sessions.iter().filter(|s| s["id"].is_string() && !put_away(s) && now.saturating_sub(s["time"]["updated"].as_u64().unwrap_or(0)) <= QUIET).collect()
}

/// The sessions open now, as the server lists them. Nothing when OpenCode is not running.
fn open() -> Option<Vec<super::Open>> {
    let list = get("/api/session?limit=40")?;
    Some(current(list["data"].as_array()?, airband_core::clock::now_ms()).into_iter().filter_map(|s| Some(super::Open { agent: agent(s["id"].as_str()?, s["location"]["directory"].as_str()), idle: s["time"]["idle"].as_u64().unwrap_or(0) >= s["time"]["busy"].as_u64().unwrap_or(0) })).collect())
}

/// The folders OpenCode has had sessions in, as its server lists them. Nothing when it is not running.
fn worked() -> Vec<super::Worked> {
    get("/api/session?limit=200").map(|list| worked_of(&list)).unwrap_or_default()
}
fn worked_of(list: &Value) -> Vec<super::Worked> {
    let found = list["data"].as_array().into_iter().flatten().filter_map(|s| Some(super::Worked { folder: s["location"]["directory"].as_str().filter(|d| !d.is_empty())?.to_string(), at: s["time"]["updated"].as_u64().unwrap_or(0) }));
    super::newest(found)
}

/// Watch OpenCode's server for as long as the tower runs: what its sessions are doing, what they say and
/// what they ask leave for. Which sessions there are is not this watcher's to say (`open`, which every
/// tool has). Blocks: run it on a thread of its own.
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
        let send = |id: &str, cwd: Option<&str>, event: Event| (tower.event)(AgentEvent { schema: SCHEMA.into(), agent: agent(id, cwd), event, via: None });
        // not the ones the tower runs itself: their driver reports for them
        let here: Vec<&Value> = current(sessions, airband_core::clock::now_ms()).into_iter().filter(|s| s["id"].as_str().is_some_and(|id| !(tower.holds)(id))).collect();
        for s in &here {
            let (Some(id), time) = (s["id"].as_str(), &s["time"]) else { continue };
            let (busy, idle) = (time["busy"].as_u64().unwrap_or(0), time["idle"].as_u64().unwrap_or(0));
            let cwd = s["location"]["directory"].as_str();
            match seen.get(id).copied() {
                None => {
                    // what was already there when the tower started has been said already: nothing of it is
                    // said again
                    let working = !first && busy > idle && send(id, cwd, Event::Working);
                    seen.insert(id.to_string(), (if first || working { busy } else { 0 }, idle));
                }
                Some((was_busy, was_idle)) => {
                    let (mut told_busy, mut told_idle) = (was_busy, was_idle);
                    if busy > was_busy && busy > idle && send(id, cwd, Event::Working) {
                        told_busy = busy;
                    }
                    if idle > was_idle && idle >= busy {
                        // its turn is over: its newest answer is what it said, and with none it only stopped
                        let ended = get(&format!("/api/session/{id}/message?limit=6")).map(|messages| newest_answer(&messages).map(|text| Event::Said { text, kind: None, turn: None }).unwrap_or(Event::Stopped { why: None }));
                        if ended.is_some_and(|event| send(id, cwd, event)) {
                            (told_busy, told_idle) = (told_busy.max(busy), idle);
                        }
                    }
                    seen.insert(id.to_string(), (told_busy, told_idle));
                }
            }
            // while it is busy it may be waiting for leave
            if busy > idle {
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
        // one that is no longer listed is forgotten here; that it has left is said by the look every tool has
        seen.retain(|id, _| here.iter().any(|s| s["id"].as_str() == Some(id.as_str())));
        first = false;
    }
}

/// His words into that session as a turn of his own.
fn deliver(session: &str, words: &str) -> Result<(), String> {
    service().ok_or_else(|| format!("{} is not running on this computer.", tool()))?;
    match post(&format!("/api/session/{session}/prompt"), json!({ "text": words })) {
        Some(200..=299) => Ok(()),
        Some(code) => Err(format!("{} did not take it ({code}).", tool())),
        None => Err(format!("{} could not be reached.", tool())),
    }
}

/// Put a session away in OpenCode (its own "archive"): it keeps what was done and stops listing it as current.
fn put_away(session: &str) -> Result<(), String> {
    let (url, password) = service().ok_or_else(|| format!("{} is not running on this computer.", tool()))?;
    match client().patch(format!("{url}/api/session/{session}")).header("authorization", auth(&password)).send_json(json!({ "archived": true })) {
        Ok(res) if (200..300).contains(&res.status().as_u16()) => Ok(()),
        Ok(res) => Err(format!("{} did not put it away ({}).", tool(), res.status().as_u16())),
        Err(_) => Err(format!("{} could not be reached.", tool())),
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
    fn open(&self) -> Option<Vec<super::Open>> {
        open()
    }
    fn put_away(&self, session: &str) -> Option<Result<(), String>> {
        Some(put_away(session))
    }
    fn puts_away(&self) -> bool {
        true
    }
    fn worked(&self) -> Vec<super::Worked> {
        worked()
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
    fn the_sessions_on_the_radio_are_the_recent_ones_not_put_away_and_each_folder_is_listed_once() {
        let hour = 3600 * 1000;
        let sessions = json!({ "data": [
            { "id": "ses_new", "time": { "updated": 10 * hour }, "location": { "directory": "/x/shop" } },
            { "id": "ses_older", "time": { "updated": 9 * hour }, "location": { "directory": "/x/shop" } },
            { "id": "ses_quiet", "time": { "updated": 7 * hour }, "location": { "directory": "/x/garden" } },
            { "id": "ses_away", "time": { "updated": 10 * hour, "archived": 10 * hour }, "location": { "directory": "/x/once" } },
        ] });
        let ids: Vec<&str> = current(sessions["data"].as_array().unwrap(), 10 * hour + 60_000).iter().filter_map(|s| s["id"].as_str()).collect();
        assert_eq!(ids, ["ses_new", "ses_older"]);
        // where it has worked: every folder, by its latest session, whether or not that one is still on the radio
        let mut worked = worked_of(&sessions);
        worked.sort_by(|a, b| a.folder.cmp(&b.folder));
        assert_eq!(worked, [super::super::Worked { folder: "/x/garden".into(), at: 7 * hour }, super::super::Worked { folder: "/x/once".into(), at: 10 * hour }, super::super::Worked { folder: "/x/shop".into(), at: 10 * hour }]);
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
