//! The driver for a coding agent that speaks the Agent Client Protocol, inside its holder (`hold.rs`).
//!
//! The Agent Client Protocol (agentclientprotocol.com) is how an editor drives a coding agent it started:
//! JSON-RPC over the agent's standard input and output. Many agents speak it, some themselves (OpenCode) and
//! some through an adapter (Codex, Claude Code). So this one driver stands for all of them, and a tool that
//! speaks it is a row: `held: { by: acp, program: […] }` (`catalog/tools/`).
//!
//! Each thing he says becomes a prompt, which the agent takes as a turn of his own; what he says while it
//! works waits here, in order. What the agent says and does goes to the tower as the same events any other
//! tool sends, under the id the tower gave it. Its requests for leave are put to him. Every turn ends with a
//! report: what it said, or that it stopped.
#![cfg(unix)]
use crate::adapters;
use crate::hold::{Held, Msg, Tell};
use airband_core::agent::{Agent, AgentEvent, Deliver, Event, SCHEMA};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::time::Duration;

/// How long the agent is given to answer one of the requests that open a session. A program fetched on first
/// use (`npx`) takes a while.
const OPENING: Duration = Duration::from_secs(120);

/// Open a session (a new one, or `session` carried on), say the agent is ready, and drive it until it ends.
/// An error is the agent never having started.
pub(crate) fn run(held: &Held, session: Option<String>, mode: Option<&str>) -> Result<(), String> {
    let send = |message: Value| held.write(&message.to_string());
    // an answer to one of this driver's own requests, waited for; what he says meanwhile is kept
    let mut early: VecDeque<Tell> = VecDeque::new();
    let mut answer_to = |wanted: u64| -> Result<Value, String> {
        loop {
            match held.rx.recv_timeout(OPENING) {
                Ok(Msg::Out(line)) => {
                    let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                    if v["id"].as_u64() == Some(wanted) && v.get("method").is_none() {
                        return match v.get("error") {
                            Some(e) => Err(e["message"].as_str().unwrap_or("the agent refused").to_string()),
                            None => Ok(v["result"].clone()),
                        };
                    }
                    // what it replays of an old session while loading it is not news
                }
                Ok(Msg::Tell(tell)) => early.push_back(tell),
                Ok(Msg::Gone) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err(held.why("it ended before it had started")),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Err(held.why("it did not answer in two minutes")),
            }
        }
    };

    send(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": 1, "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false }, "clientInfo": { "name": "airband", "version": env!("CARGO_PKG_VERSION") } } }));
    let about = answer_to(1)?;
    let (session, made) = match session {
        // carried on, or not at all: a new session under an old agent's name would be a different agent
        Some(session) => {
            if about["agentCapabilities"]["loadSession"] != true {
                return Err("this agent cannot carry a session on".into());
            }
            send(json!({ "jsonrpc": "2.0", "id": 2, "method": "session/load", "params": { "sessionId": session, "cwd": held.cwd, "mcpServers": [] } }));
            let loaded = answer_to(2).map_err(|e| format!("its session could not be carried on: {e}"))?;
            (session, loaded)
        }
        None => {
            send(json!({ "jsonrpc": "2.0", "id": 3, "method": "session/new", "params": { "cwd": held.cwd, "mcpServers": [] } }));
            let made = answer_to(3)?;
            (made["sessionId"].as_str().ok_or("the agent opened no session")?.to_string(), made)
        }
    };
    // the mode its launcher asks for, when the agent does not open in it: refused, it is not an agent he asked for
    if let Some(mode) = mode.filter(|m| made["modes"]["currentModeId"] != **m) {
        send(json!({ "jsonrpc": "2.0", "id": 4, "method": "session/set_mode", "params": { "sessionId": session, "modeId": mode } }));
        answer_to(4).map_err(|e| format!("it would not be put in its \"{mode}\" mode: {e}"))?;
    }
    let me = Agent { id: held.id.clone(), harness: held.harness.clone(), cwd: Some(held.cwd.clone()), name: None, title: None, pid: None, deliver: Some(Deliver::Held), meta: Default::default() };
    let tell = |event: Event| {
        adapters::post(&AgentEvent { schema: SCHEMA.into(), agent: me.clone(), event });
    };
    held.ready(Some(session.clone()));
    tell(Event::Checkin);
    // what it runs on, where the agent says: its own name for the model in use
    let model = made["models"]["currentModelId"].as_str().or_else(|| made["configOptions"].as_array().and_then(|o| o.iter().find(|o| o["category"] == "model")).and_then(|o| o["currentValue"].as_str()));
    if let Some(model) = model {
        tell(Event::Model { name: model.to_string(), effort: None });
    }

    // one prompt at a time: what he says while it works waits here, in the order he said it
    let (mut waiting, mut running, mut said, mut next_id) = (VecDeque::<String>::new(), None::<u64>, String::new(), 10u64);
    let mut early: VecDeque<Msg> = early.into_iter().map(Msg::Tell).collect();
    loop {
        let msg = match early.pop_front() {
            Some(msg) => msg,
            None => match held.rx.recv() {
                Ok(msg) => msg,
                Err(_) => break,
            },
        };
        match msg {
            Msg::Tell(Tell::Say(text)) => waiting.push_back(text),
            Msg::Tell(Tell::Stop) => {
                if running.is_some() {
                    send(json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": session } }));
                }
            }
            Msg::Tell(Tell::End) => {
                held.close_input();
                break;
            }
            Msg::Out(line) => {
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                match (v["method"].as_str(), v["id"].clone()) {
                    (Some("session/update"), _) => {
                        let update = &v["params"]["update"];
                        match update["sessionUpdate"].as_str() {
                            Some("agent_message_chunk") => said.push_str(update["content"]["text"].as_str().unwrap_or_default()),
                            // what it is doing right now, in the agent's own few words for the step
                            Some("tool_call") => {
                                // what it writes after a step is a new paragraph of what it says
                                if !said.trim_end().is_empty() && !said.ends_with("\n\n") {
                                    said = format!("{}\n\n", said.trim_end());
                                }
                                if let Some(title) = update["title"].as_str().filter(|t| !t.trim().is_empty()) {
                                    tell(Event::Activity { text: title.chars().take(80).collect() });
                                }
                            }
                            _ => {}
                        }
                    }
                    // it asks leave to do something: he is asked, and his answer picks one of the choices it gave
                    (Some("session/request_permission"), request) if !request.is_null() => {
                        let (params, me, to_program) = (v["params"].clone(), me.clone(), held.to_program.clone());
                        std::thread::spawn(move || {
                            let outcome = leave(&params, me).and_then(|ask| adapters::ask_him(&ask)).and_then(|(allow, _)| option(&params, allow));
                            let answer = json!({ "jsonrpc": "2.0", "id": request, "result": { "outcome": match outcome {
                                Some(option) => json!({ "outcome": "selected", "optionId": option }),
                                // No answer chooses nothing: waiting never becomes a yes.
                                None => json!({ "outcome": "cancelled" }),
                            } } });
                            if let Some(to) = to_program.lock().unwrap().as_mut() {
                                use std::io::Write;
                                let _ = writeln!(to, "{answer}").and_then(|()| to.flush());
                            }
                        });
                    }
                    // anything else it asks of its client is something this one said it does not do
                    (Some(_), request) if !request.is_null() => send(json!({ "jsonrpc": "2.0", "id": request, "error": { "code": -32601, "message": "not supported by this client" } })),
                    // the prompt has ended. What it wrote in that turn is what it said; with nothing written
                    // it stopped, and when the turn failed the agent's own words say why.
                    (None, done) if done.as_u64().is_some() && done.as_u64() == running => {
                        running = None;
                        let text = std::mem::take(&mut said);
                        tell(match (text.trim(), v.get("error")) {
                            ("", Some(e)) => Event::Stopped { why: Some(e["message"].as_str().unwrap_or("the agent gave no reason").to_string()) },
                            ("", None) => Event::Stopped { why: None },
                            (text, _) => Event::Said { text: text.to_string(), kind: None },
                        });
                    }
                    _ => {}
                }
            }
            Msg::Gone => break,
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
    tell(Event::Signoff);
    Ok(())
}

/// A request for leave of the agent's, as he is asked it. For a command (the protocol's `execute` kind, with
/// the command in its input) what he reads is the command itself; for anything else, the agent's own title
/// for what it wants to do. With it goes everything the agent said about the request. Nothing when it is too
/// long to put in front of him whole.
fn leave(params: &Value, agent: Agent) -> Option<airband_core::agent::PermissionAsk> {
    let call = &params["toolCall"];
    let title = call["title"].as_str().map(str::trim).filter(|t| !t.is_empty()).unwrap_or("use a tool");
    let command = match &call["rawInput"]["command"] {
        Value::String(line) => Some(line.trim().to_string()),
        Value::Array(parts) => Some(parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")),
        _ => None,
    };
    let what = match command.filter(|c| !c.is_empty() && call["kind"] == "execute") {
        Some(command) => format!("run `{command}`"),
        None => title.chars().next().map(|c| c.to_lowercase().collect::<String>() + &title[c.len_utf8()..]).unwrap_or_default(),
    };
    adapters::hooks::leave(agent, title, what, &call["rawInput"], "", None)
}

/// The one of the agent's own choices that is his answer: this once, never for always.
fn option(params: &Value, allow: bool) -> Option<String> {
    let kind = if allow { "allow_once" } else { "reject_once" };
    params["options"].as_array().and_then(|o| o.iter().find(|o| o["kind"] == kind)).and_then(|o| o["optionId"].as_str()).map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn his_answer_picks_one_of_the_agents_own_choices_and_only_for_this_once() {
        let params = json!({ "toolCall": { "title": "Run echo hi", "rawInput": { "command": "echo hi" } }, "options": [
            { "optionId": "always", "kind": "allow_always" }, { "optionId": "yes", "kind": "allow_once" }, { "optionId": "no", "kind": "reject_once" }] });
        assert_eq!(option(&params, true).as_deref(), Some("yes"));
        assert_eq!(option(&params, false).as_deref(), Some("no"));
        // an agent that offers no "this once" is not allowed for always in its place
        assert_eq!(option(&json!({ "options": [{ "optionId": "always", "kind": "allow_always" }] }), true), None);
        let me = Agent { id: "a".into(), harness: "codex".into(), cwd: None, name: None, title: None, pid: None, deliver: Some(Deliver::Held), meta: Default::default() };
        let ask = leave(&params, me.clone()).unwrap();
        assert_eq!(ask.what, "run echo hi");
        let command = json!({ "toolCall": { "title": "echo hi", "kind": "execute", "rawInput": { "command": ["bash", "-lc", "echo hi"] } } });
        assert_eq!(leave(&command, me.clone()).unwrap().what, "run `bash -lc echo hi`");
        assert!(ask.detail.contains("echo hi"));
        assert!(leave(&json!({ "toolCall": { "title": "x".repeat(5000) } }), me).is_none());
    }
}
