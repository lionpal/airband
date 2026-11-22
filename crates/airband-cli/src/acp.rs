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
//!
//! What a session can be set to is the agent's to say. It lists its settings when the session opens
//! (`configOptions`), each with the protocol's own word for what kind of setting it is (`category`): `model`
//! is what it runs on and `thought_level` how hard it thinks. Those two lists go to the tower as they are
//! (`hold::State::Ready`), and the model and effort he asked for are set from them before the agent is said
//! to be there.
#![cfg(unix)]
use crate::adapters;
use crate::hold::{Held, Msg, Tell};
use airband_core::agent::{Agent, AgentEvent, Deliver, Event, SCHEMA};
use airband_core::tools::{Choice, Offers};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::time::Duration;

/// How long the agent is given to answer one of the requests that open a session. A program fetched on first
/// use (`npx`) takes a while.
const OPENING: Duration = Duration::from_secs(120);

/// What he asked a new agent to be set to, in his own words for it: a name the agent lists, or its id.
#[derive(Default)]
pub(crate) struct Asked {
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// The protocol's words for the two kinds of setting airband sets (a setting's `category`).
const MODEL: &str = "model";
const EFFORT: &str = "thought_level";

/// One of the settings a session lists, by the protocol's word for its kind: its id, the value it has now,
/// and the values it can be given (listed flat, or in named groups).
struct Setting {
    id: String,
    now: Option<String>,
    choices: Vec<Choice>,
}
fn setting(options: &Value, category: &str) -> Option<Setting> {
    let o = options.as_array()?.iter().find(|o| o["category"] == category && o["type"] == "select")?;
    let choice = |c: &Value| Some(Choice { id: c["value"].as_str()?.to_string(), name: c["name"].as_str().unwrap_or_default().to_string() });
    let choices = o["options"].as_array().map(|list| list.iter().flat_map(|c| match c["options"].as_array() {
        Some(group) => group.iter().filter_map(choice).collect::<Vec<_>>(),
        None => choice(c).into_iter().collect(),
    }).collect()).unwrap_or_default();
    Some(Setting { id: o["id"].as_str()?.to_string(), now: o["currentValue"].as_str().map(String::from), choices })
}

/// What a session's settings say it can be set to.
fn offers(options: &Value) -> Offers {
    let of = |category| setting(options, category).map(|s| s.choices).unwrap_or_default();
    Offers { models: of(MODEL), efforts: of(EFFORT) }
}

/// Why something he asked for cannot be had: the agent has no such setting, or no such value of it. The ones
/// it does have are named when they are few enough to say.
/// `noun` is the setting's name ("effort"), `what` the same with its article ("an effort").
fn no_such(noun: &str, what: &str, said: &str, has: Option<&Setting>) -> String {
    match has.map(|s| s.choices.as_slice()) {
        None | Some([]) => format!("it cannot be given {what}"),
        Some(choices) if choices.len() <= 8 => format!("it has no {noun} called {said}. It can be {}", choices.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join(", ")),
        Some(_) => format!("it has no {noun} called {said}"),
    }
}

/// Open a session (a new one, or `session` carried on), set it to what he asked for, say the agent is ready,
/// and drive it until it ends. An error is the agent never having started.
pub(crate) fn run(held: &Held, session: Option<String>, mode: Option<&str>, asked: Asked) -> Result<(), String> {
    let mut opening = Opening { held, early: VecDeque::new() };
    let (session, made) = opening.open(session, mode)?;
    let me = Agent { id: held.id.clone(), harness: held.harness.clone(), cwd: Some(held.cwd.clone()), name: None, title: None, pid: None, deliver: Some(Deliver::Held), meta: Default::default() };
    // What it can be set to, as it opens. Then what he asked for.
    let offered = offers(&made["configOptions"]);
    let settings = opening.set_asked(&session, made["configOptions"].clone(), &asked)?;
    held.ready(Some(session.clone()), offered);
    post(&me, Event::Checkin);
    // what it runs on, where the agent says: its own id for the model in use, and for how hard it thinks
    let now = |category| setting(&settings, category).and_then(|s| s.now);
    if let Some(model) = now(MODEL) {
        post(&me, Event::Model { name: model, effort: now(EFFORT) });
    }
    drive(held, &session, &me, opening.early);
    post(&me, Event::Signoff);
    Ok(())
}

/// Tell the tower what the agent did.
fn post(me: &Agent, event: Event) {
    adapters::post(&AgentEvent { schema: SCHEMA.into(), agent: me.clone(), event, via: None });
}

fn send(held: &Held, message: Value) {
    held.write(&message.to_string());
}

/// The agent while it opens: the answers to this driver's own requests are waited for, and what he says
/// meanwhile is kept for when it is ready.
struct Opening<'a> {
    held: &'a Held,
    early: VecDeque<Tell>,
}

impl Opening<'_> {
    /// The answer to one of this driver's own requests.
    fn answer_to(&mut self, wanted: u64) -> Result<Value, String> {
        loop {
            match self.held.rx.recv_timeout(OPENING) {
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
                Ok(Msg::Tell(tell)) => self.early.push_back(tell),
                Ok(Msg::Gone) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err(self.held.why("it ended before it had started")),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Err(self.held.why("it did not answer in two minutes")),
            }
        }
    }

    /// The session opened, or carried on, and in the mode its launcher asks for: its id, and what the agent
    /// answered on opening it.
    fn open(&mut self, session: Option<String>, mode: Option<&str>) -> Result<(String, Value), String> {
        let held = self.held;
        send(held, json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": 1, "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false }, "clientInfo": { "name": "airband", "version": env!("CARGO_PKG_VERSION") } } }));
        let about = self.answer_to(1)?;
        let (session, made) = match session {
            // carried on, or not at all: a new session under an old agent's name would be a different agent
            Some(session) => {
                if about["agentCapabilities"]["loadSession"] != true {
                    return Err("this agent cannot carry a session on".into());
                }
                send(held, json!({ "jsonrpc": "2.0", "id": 2, "method": "session/load", "params": { "sessionId": session, "cwd": held.cwd, "mcpServers": [] } }));
                let loaded = self.answer_to(2).map_err(|e| format!("its session could not be carried on: {e}"))?;
                (session, loaded)
            }
            None => {
                send(held, json!({ "jsonrpc": "2.0", "id": 3, "method": "session/new", "params": { "cwd": held.cwd, "mcpServers": [] } }));
                let made = self.answer_to(3)?;
                (made["sessionId"].as_str().ok_or("the agent opened no session")?.to_string(), made)
            }
        };
        // the mode its launcher asks for, when the agent does not open in it: refused, it is not an agent he asked for
        if let Some(mode) = mode.filter(|m| made["modes"]["currentModeId"] != **m) {
            send(held, json!({ "jsonrpc": "2.0", "id": 4, "method": "session/set_mode", "params": { "sessionId": session, "modeId": mode } }));
            self.answer_to(4).map_err(|e| format!("it would not be put in its \"{mode}\" mode: {e}"))?;
        }
        Ok((session, made))
    }

    /// What he asked for, the model first: which efforts there are can depend on it, and the agent lists its
    /// settings again after each change. One it does not have is not left out and the agent started without
    /// it. Gives its settings as they now are.
    fn set_asked(&mut self, session: &str, mut settings: Value, asked: &Asked) -> Result<Value, String> {
        for (category, noun, what, said, request) in [(MODEL, "model", "a model", &asked.model, 5), (EFFORT, "effort", "an effort", &asked.effort, 6)] {
            let Some(said) = said else { continue };
            let has = setting(&settings, category);
            let (Some(s), Some(choice)) = (has.as_ref(), has.as_ref().and_then(|s| s.choices.iter().find(|c| c.id == *said || c.name == *said))) else {
                return Err(no_such(noun, what, said, has.as_ref()));
            };
            if s.now.as_deref() != Some(choice.id.as_str()) {
                send(self.held, json!({ "jsonrpc": "2.0", "id": request, "method": "session/set_config_option", "params": { "sessionId": session, "configId": s.id, "value": choice.id } }));
                let set = self.answer_to(request).map_err(|e| format!("it would not be set to {}: {e}", choice.id))?;
                if !set["configOptions"].is_array() {
                    return Err(format!("it did not say whether it was set to {}", choice.id));
                }
                settings = set["configOptions"].clone();
            }
        }
        Ok(settings)
    }
}

/// Drive the open session until it ends: one prompt at a time, what he says while it works waiting here in the
/// order he said it.
fn drive(held: &Held, session: &str, me: &Agent, early: VecDeque<Tell>) {
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
            Msg::Tell(Tell::Say(text, _)) => waiting.push_back(text),
            Msg::Tell(Tell::Stop) => {
                if running.is_some() {
                    send(held, json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": session } }));
                }
            }
            Msg::Tell(Tell::End) => {
                held.close_input();
                break;
            }
            Msg::Out(line) => {
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                match (v["method"].as_str(), v["id"].clone()) {
                    (Some("session/update"), _) => on_update(&v["params"]["update"], &mut said, me),
                    // it asks leave to do something: he is asked, and his answer picks one of the choices it gave
                    (Some("session/request_permission"), request) if !request.is_null() => ask_leave(held, v["params"].clone(), request, me.clone()),
                    // anything else it asks of its client is something this one said it does not do
                    (Some(_), request) if !request.is_null() => send(held, json!({ "jsonrpc": "2.0", "id": request, "error": { "code": -32601, "message": "not supported by this client" } })),
                    // the prompt has ended. What it wrote in that turn is what it said; with nothing written
                    // it stopped, and when the turn failed the agent's own words say why.
                    (None, done) if done.as_u64().is_some() && done.as_u64() == running => {
                        running = None;
                        let text = std::mem::take(&mut said);
                        post(me, match (text.trim(), v.get("error")) {
                            ("", Some(e)) => Event::Stopped { why: Some(e["message"].as_str().unwrap_or("the agent gave no reason").to_string()) },
                            ("", None) => Event::Stopped { why: None },
                            (text, _) => Event::Said { text: text.to_string(), kind: None, turn: None },
                        });
                    }
                    _ => {}
                }
            }
            Msg::Gone => break,
        }
        if running.is_none()
            && let Some(text) = waiting.pop_front() {
                next_id += 1;
                running = Some(next_id);
                said.clear();
                send(held, json!({ "jsonrpc": "2.0", "id": next_id, "method": "session/prompt", "params": { "sessionId": session, "prompt": [{ "type": "text", "text": text }] } }));
                post(me, Event::Working);
            }
    }
}

/// Part of a turn as it goes: words of what it says, or a step it is taking.
fn on_update(update: &Value, said: &mut String, me: &Agent) {
    match update["sessionUpdate"].as_str() {
        Some("agent_message_chunk") => said.push_str(update["content"]["text"].as_str().unwrap_or_default()),
        // what it is doing right now, in the agent's own few words for the step
        Some("tool_call") => {
            // what it writes after a step is a new paragraph of what it says
            if !said.trim_end().is_empty() && !said.ends_with("\n\n") {
                *said = format!("{}\n\n", said.trim_end());
            }
            if let Some(title) = update["title"].as_str().filter(|t| !t.trim().is_empty()) {
                post(me, Event::Activity { text: title.chars().take(80).collect() });
            }
        }
        _ => {}
    }
}

/// He is asked for leave, off this thread, and his answer goes back as one of the choices the agent gave.
fn ask_leave(held: &Held, params: Value, request: Value, me: Agent) {
    let to_program = held.to_program.clone();
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

    #[test]
    fn what_a_session_can_be_set_to_is_what_the_agent_lists_under_the_protocols_own_words() {
        let options = json!([
            { "id": "m", "category": "model", "type": "select", "currentValue": "one", "options": [{ "value": "one", "name": "One" }, { "group": "g", "name": "More", "options": [{ "value": "two", "name": "Two" }] }] },
            { "id": "reasoning", "category": "thought_level", "type": "select", "currentValue": "low", "options": [{ "value": "low", "name": "Low" }, { "value": "high", "name": "High" }] },
            { "id": "mode", "category": "mode", "type": "select", "currentValue": "plan", "options": [{ "value": "plan", "name": "Plan" }] },
            { "id": "model-ish", "name": "Model", "type": "select", "currentValue": "x", "options": [{ "value": "x", "name": "X" }] }
        ]);
        let o = offers(&options);
        assert_eq!(o.models.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["one", "two"]);
        assert_eq!(o.efforts.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Low", "High"]);
        let s = setting(&options, EFFORT).unwrap();
        assert_eq!((s.id.as_str(), s.now.as_deref()), ("reasoning", Some("low")));
        // an agent that lists nothing offers nothing
        assert_eq!(offers(&Value::Null), Offers::default());
        assert_eq!(no_such("effort", "an effort", "huge", Some(&s)), "it has no effort called huge. It can be low, high");
        assert_eq!(no_such("model", "a model", "x", None), "it cannot be given a model");
    }
}
