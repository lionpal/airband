//! `airband report`: the adapter for everything that has no adapter of its own. Any script can put an agent on
//! the radio with it.
use super::post;
use airband_core::agent::{Agent, AgentEvent, Deliver, Event, SCHEMA};
use std::io::Read;

/// `airband report`: the adapter for everything else. Any script can put an agent on the radio:
///
///   airband report --id build-7 --harness codex --cwd . --tmux %3 said "Tests pass. Ship it?"
///   echo "long message" | airband report --id build-7 --harness codex said -
///   airband report --id build-7 --harness codex show chart.png      (a picture for him, shown with what it says next)
///   airband report --id build-7 --harness codex permission "run the production migration"   (waits for his yes or no)
///   airband report --id build-7 --harness codex signoff
///
/// `--meta key=value` (repeatable) records anything else about the agent: its model, a branch, a ticket.
pub fn run(args: &[String]) -> Result<(), String> {
    let mut agent = Agent { id: String::new(), harness: "custom".into(), cwd: std::env::current_dir().ok().map(|p| p.display().to_string()), name: None, title: None, pid: None, deliver: None, meta: Default::default() };
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |flag: &str| it.next().cloned().ok_or(format!("{flag} needs a value"));
        match a.as_str() {
            "--id" => agent.id = value("--id")?,
            "--harness" => agent.harness = value("--harness")?,
            "--cwd" => agent.cwd = Some(std::path::absolute(value("--cwd")?).map_err(|e| e.to_string())?.display().to_string()),
            "--name" => agent.name = Some(value("--name")?),
            "--title" => agent.title = Some(value("--title")?),
            "--pid" => agent.pid = Some(value("--pid")?.parse().map_err(|_| "--pid needs a number")?),
            "--meta" => {
                let pair = value("--meta")?;
                let (k, v) = pair.split_once('=').ok_or("--meta takes key=value")?;
                agent.meta.insert(k.to_string(), v.into());
            }
            "--tmux" => agent.deliver = Some(Deliver::Keys { tmux: value("--tmux")? }),
            // the tower started this agent and holds its input
            "--held" => agent.deliver = Some(Deliver::Held),
            "--deliver-command" => agent.deliver = Some(Deliver::Command { argv: value("--deliver-command")?.split_whitespace().map(String::from).collect() }),
            _ => rest.push(a.clone()),
        }
    }
    if agent.id.is_empty() {
        return Err("--id is required: any name that stays the same for this agent's session".into());
    }
    let body = || -> Result<String, String> {
        let words = rest[1..].join(" ");
        if words == "-" {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s).map_err(|e| e.to_string())?;
            return Ok(s.trim().to_string());
        }
        if words.is_empty() { Err("say what it said: text, or - to read standard input".into()) } else { Ok(words) }
    };
    // Asking permission is not an event: it waits for his answer and says it with the exit status, so a script
    // can write  `airband report --id build-7 permission "run the production migration" && ./migrate`.
    //   0 he allowed it; 1 he refused (his reason, if any, is printed); 2 no answer: ask some other way.
    if rest.first().map(String::as_str) == Some("permission") {
        let what = rest[1..].join(" ");
        if what.is_empty() {
            return Err("say what it wants to do: permission \"run the production migration\"".into());
        }
        let ask = airband_core::agent::PermissionAsk { schema: SCHEMA.into(), agent, what, detail: String::new(), question: None };
        match super::ask_him(&ask) {
            Some((true, _)) => return Ok(()),
            Some((false, reason)) => {
                if !reason.is_empty() {
                    println!("{reason}");
                }
                std::process::exit(1);
            }
            None => std::process::exit(2),
        }
    }
    let event = match rest.first().map(String::as_str) {
        Some("checkin") => Event::Checkin,
        Some("working") => Event::Working,
        Some("said") => Event::Said { text: body()?, kind: None, turn: None },
        Some("asked") => Event::Said { text: body()?, kind: Some(airband_core::radio::Kind::NeedsYou), turn: None },
        Some("notice") => Event::Notice { text: body()? },
        // a picture goes with the next thing the agent says: `show` for one he should see, `picture` for one it only looked at
        Some(word @ ("picture" | "show")) => Event::Picture { path: std::path::absolute(rest.get(1).ok_or("give the picture's file")?).map_err(|e| e.to_string())?.display().to_string(), shown: word == "show" },
        Some("doing") => Event::Activity { text: body()? },
        // what it runs on: `model "Opus 4.7" high`; the effort is left off where the harness has none
        Some("model") => Event::Model { name: rest.get(1).ok_or("give the model's name")?.clone(), effort: rest.get(2).cloned() },
        Some("signoff") => Event::Signoff,
        _ => return Err("expected one of: checkin, working, doing <text>, model <name> [effort], said <text>, asked <text>, notice <text>, show <picture file>, picture <picture file>, signoff".into()),
    };
    if post(&AgentEvent { schema: SCHEMA.into(), agent, event }) { Ok(()) } else { Err("airband did not answer (is it running? `airband status`)".into()) }
}
