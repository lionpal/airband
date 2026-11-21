//! `airband acp`: runs one coding agent that speaks the Agent Client Protocol, for the tower.
//!
//! The Agent Client Protocol (agentclientprotocol.com) is how an editor drives a coding agent it started:
//! JSON-RPC over the agent's standard input and output. Many agents speak it, some themselves (OpenCode) and
//! some through an adapter (Codex, Claude Code). So one program here stands for all of them:
//!
//!   airband acp <harness> <id> <program> [arguments...] [--carry-on]
//!
//! It is what a launcher of the held kind runs (`tower/held.rs`), under a holder like any other held agent.
//! His words come in on standard input, a line each (`{"say": "…"}`), and each becomes a prompt, which the
//! agent takes as a turn of his own. `{"stop": …}` asks it to leave off. What the agent says and does goes to
//! the tower as the same events any other tool sends, under `<id>`. Its requests for leave are put to him.
//! `--carry-on` loads the session this id had before, when the agent can.
#![cfg(unix)]
use crate::adapters;
use airband_core::agent::{Agent, AgentEvent, Deliver, Event, PermissionAsk, SCHEMA};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

enum Msg {
    FromAgent(Value),
    FromHim(Value),
    AgentGone,
    InputClosed,
}

/// Where what this id's session is called by its agent is kept, so it can be carried on later.
fn kept(id: &str) -> std::path::PathBuf {
    crate::config::dir().join("acp").join(format!("{id}.json"))
}

/// The sessions agents started here call their own: a tool that is also watched from outside (OpenCode's
/// server shows every session) leaves these to this program.
pub fn owned() -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(crate::config::dir().join("acp")) else { return Vec::new() };
    dir.flatten().filter_map(|f| std::fs::read(f.path()).ok()).filter_map(|b| serde_json::from_slice::<Value>(&b).ok()).filter_map(|v| v["session"].as_str().map(String::from)).collect()
}

pub fn run(args: &[String]) -> Result<(), String> {
    let (harness, id) = (args.first().ok_or("usage: airband acp <harness> <id> <program> [arguments...]")?.clone(), args.get(1).ok_or("airband acp needs an id")?.clone());
    let mut argv: Vec<String> = args[2..].to_vec();
    let carry_on = argv.last().map(String::as_str) == Some("--carry-on");
    if carry_on {
        argv.pop();
    }
    let (program, rest) = argv.split_first().ok_or("airband acp needs a program to run")?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?.display().to_string();
    // The agent's own hooks, if it has any, stay quiet: this program reports for it, and two reports of the
    // same thing would be read to him twice.
    let mut child = Command::new(program).args(rest).env("AIRBAND_INTERNAL", "1").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("{program} did not start: {e}"))?;
    let to_agent = Arc::new(Mutex::new(child.stdin.take().unwrap()));
    let send = {
        let to_agent = to_agent.clone();
        move |message: Value| {
            let mut out = to_agent.lock().unwrap();
            let _ = writeln!(out, "{message}").and_then(|()| out.flush());
        }
    };
    let (tx, rx) = mpsc::channel::<Msg>();
    {
        let (tx, out) = (tx.clone(), child.stdout.take().unwrap());
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    let _ = tx.send(Msg::FromAgent(v));
                }
            }
            let _ = tx.send(Msg::AgentGone);
        });
    }
    let me = Agent { id: id.clone(), harness, cwd: Some(cwd.clone()), name: None, title: None, pid: None, deliver: Some(Deliver::Held), meta: Default::default() };
    let tell = {
        let me = me.clone();
        move |event: Event| {
            adapters::post(&AgentEvent { schema: SCHEMA.into(), agent: me.clone(), event });
        }
    };
    // an answer to one of this program's own requests, waited for; anything else that arrives meanwhile is kept
    let mut early: VecDeque<Msg> = VecDeque::new();
    let answer_to = |wanted: u64, early: &mut VecDeque<Msg>| -> Result<Value, String> {
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(120)) {
                Ok(Msg::FromAgent(v)) if v["id"].as_u64() == Some(wanted) && v.get("method").is_none() => {
                    return match v.get("error") {
                        Some(e) => Err(e["message"].as_str().unwrap_or("the agent refused").to_string()),
                        None => Ok(v["result"].clone()),
                    };
                }
                // what it replays of an old session while loading it is not news
                Ok(Msg::FromAgent(v)) if v["method"] == "session/update" => {}
                Ok(other) => early.push_back(other),
                Err(_) => return Err("the agent did not answer".into()),
            }
        }
    };

    send(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": 1, "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false }, "clientInfo": { "name": "airband", "version": env!("CARGO_PKG_VERSION") } } }));
    let about = answer_to(1, &mut early)?;
    let was = carry_on.then(|| std::fs::read(kept(&id)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v["session"].as_str().map(String::from))).flatten();
    let session = match was.filter(|_| about["agentCapabilities"]["loadSession"] == true) {
        Some(session) => {
            send(json!({ "jsonrpc": "2.0", "id": 2, "method": "session/load", "params": { "sessionId": session, "cwd": cwd, "mcpServers": [] } }));
            answer_to(2, &mut early).map(|_| session)
        }
        None => Err("nothing to carry on".into()),
    };
    let (session, made) = match session {
        Ok(session) => (session, Value::Null),
        Err(_) => {
            send(json!({ "jsonrpc": "2.0", "id": 3, "method": "session/new", "params": { "cwd": cwd, "mcpServers": [] } }));
            let made = answer_to(3, &mut early)?;
            (made["sessionId"].as_str().ok_or("the agent opened no session")?.to_string(), made)
        }
    };
    let _ = std::fs::create_dir_all(crate::config::dir().join("acp"));
    let _ = std::fs::write(kept(&id), json!({ "session": session }).to_string());
    tell(Event::Checkin);
    // what it runs on, where the agent says: its own name for the model in use
    let model = made["models"]["currentModelId"].as_str().or_else(|| made["configOptions"].as_array().and_then(|o| o.iter().find(|o| o["category"] == "model")).and_then(|o| o["currentValue"].as_str()));
    if let Some(model) = model {
        tell(Event::Model { name: model.to_string(), effort: None });
    }

    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    let _ = tx.send(Msg::FromHim(v));
                }
            }
            let _ = tx.send(Msg::InputClosed);
        });
    }

    // one prompt at a time: what he says while it works waits here, in the order he said it
    let (mut waiting, mut running, mut said, mut next_id) = (VecDeque::<String>::new(), None::<u64>, String::new(), 10u64);
    loop {
        let msg = match early.pop_front() {
            Some(msg) => msg,
            None => match rx.recv() {
                Ok(msg) => msg,
                Err(_) => break,
            },
        };
        match msg {
            Msg::FromHim(v) => {
                if let Some(text) = v["say"].as_str() {
                    waiting.push_back(text.to_string());
                } else if v.get("stop").is_some() && running.is_some() {
                    send(json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": session } }));
                }
            }
            Msg::FromAgent(v) => match (v["method"].as_str(), v["id"].clone()) {
                (Some("session/update"), _) => {
                    let update = &v["params"]["update"];
                    match update["sessionUpdate"].as_str() {
                        Some("agent_message_chunk") => said.push_str(update["content"]["text"].as_str().unwrap_or_default()),
                        // what it is doing right now, in the agent's own few words for the step
                        Some("tool_call") => {
                            if let Some(title) = update["title"].as_str().filter(|t| !t.trim().is_empty()) {
                                tell(Event::Activity { text: title.chars().take(80).collect() });
                            }
                        }
                        _ => {}
                    }
                }
                // it asks leave to do something: he is asked, and his answer picks one of the choices it gave
                (Some("session/request_permission"), request) if !request.is_null() => {
                    let (params, me, send) = (v["params"].clone(), me.clone(), send.clone());
                    std::thread::spawn(move || {
                        let call = &params["toolCall"];
                        let title = call["title"].as_str().map(str::trim).filter(|t| !t.is_empty()).unwrap_or("use a tool");
                        let what = title.chars().next().map(|c| c.to_lowercase().collect::<String>() + &title[c.len_utf8()..]).unwrap_or_default();
                        let detail = serde_json::to_string_pretty(&call["rawInput"]).unwrap_or_default();
                        let answer = adapters::ask_him(&PermissionAsk { schema: SCHEMA.into(), agent: me, what, detail: format!("{title}\n{}", detail.chars().take(6000).collect::<String>()), question: None });
                        // one of the agent's own choices, of the kind his answer was. No answer chooses nothing:
                        // waiting never becomes a yes.
                        let pick = |kind: &str| params["options"].as_array().and_then(|o| o.iter().find(|o| o["kind"] == kind)).and_then(|o| o["optionId"].as_str()).map(String::from);
                        let outcome = match answer {
                            Some((true, _)) => pick("allow_once"),
                            Some((false, _)) => pick("reject_once"),
                            None => None,
                        };
                        send(json!({ "jsonrpc": "2.0", "id": request, "result": { "outcome": match outcome {
                            Some(option) => json!({ "outcome": "selected", "optionId": option }),
                            None => json!({ "outcome": "cancelled" }),
                        } } }));
                    });
                }
                // anything else it asks of its client is something this one said it does not do
                (Some(_), request) if !request.is_null() => send(json!({ "jsonrpc": "2.0", "id": request, "error": { "code": -32601, "message": "not supported by this client" } })),
                // the prompt has ended: what it wrote in that turn is what it said
                (None, done) if done.as_u64().is_some() && done.as_u64() == running => {
                    running = None;
                    let text = std::mem::take(&mut said);
                    if !text.trim().is_empty() {
                        tell(Event::Said { text: text.trim().to_string(), kind: None });
                    }
                }
                _ => {}
            },
            Msg::AgentGone => break,
            Msg::InputClosed => {
                let _ = child.kill();
                break;
            }
        }
        if running.is_none() {
            if let Some(text) = waiting.pop_front() {
                next_id += 1;
                running = Some(next_id);
                said.clear();
                send(json!({ "jsonrpc": "2.0", "id": next_id, "method": "session/prompt", "params": { "sessionId": session, "prompt": [{ "type": "text", "text": text }] } }));
                tell(Event::Working);
            }
        }
    }
    let _ = child.wait();
    let _ = std::fs::remove_file(kept(&id));
    tell(Event::Signoff);
    Ok(())
}
