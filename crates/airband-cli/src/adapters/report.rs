//! `airband report`: the adapter for everything that has no adapter of its own. Any script can put an agent on
//! the radio with it.
use super::{Posted, post};
use airband_core::agent::{AgentEvent, Deliver, Event, SCHEMA};
use std::io::Read;

/// `airband report`: the adapter for everything else. Any script can put an agent on the radio:
///
///   airband report --id build-7 --harness my-tool --cwd . --terminal tmux --pane %3 said "Tests pass. Ship it?"
///   echo "long message" | airband report --id build-7 --harness my-tool said -
///   airband report --id build-7 --harness my-tool show chart.png      (a picture for him, shown with what it says next)
///   airband report --id build-7 --harness my-tool permission "run the production migration"   (waits for his yes or no)
///   airband report --id build-7 --harness my-tool --within 120 permission "run it"   (the script stops waiting after 120 seconds)
///   airband report --id build-7 --harness my-tool made https://example.com/preview/7 "Checkout preview"   (a card for it)
///   airband report --id build-7 --harness my-tool signoff
///
/// `--meta key=value` (repeatable) records anything else about the agent: its model, a branch, a ticket.
pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let mut agent = airband_core::agent::Agent { cwd: std::env::current_dir().ok().map(|p| p.display().to_string()), ..crate::adapters::agent_with(String::new(), String::new()) };
    let mut rest = Vec::new();
    let (mut terminal, mut pane) = (None, None);
    // how long a script waits on a request for leave before it settles it itself, if it says
    let mut within: Option<u64> = None;
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
            "--within" => within = Some(value("--within")?.parse().ok().filter(|s| *s > 0).ok_or("--within needs a number of seconds")?),
            "--terminal" => terminal = Some(value("--terminal")?),
            "--pane" => pane = Some(value("--pane")?),
            // the tower started this agent and holds its input
            "--held" => agent.deliver = Some(Deliver::Held),
            // the program and its words as a JSON list, so a word may hold spaces or quotes
            "--deliver-command" => {
                let argv: Vec<String> = serde_json::from_str(&value("--deliver-command")?).map_err(|_| "--deliver-command takes the program and its words as a JSON list: [\"notify\", \"--to\", \"me\"]".to_string())?;
                if argv.is_empty() {
                    return Err("--deliver-command needs a program to run".into());
                }
                agent.deliver = Some(Deliver::Command { argv });
            }
            _ => rest.push(a.clone()),
        }
    }
    match (terminal, pane) {
        (Some(terminal), Some(pane)) => {
            if airband_core::catalog::terminal(&terminal).is_none() {
                let known: Vec<String> = airband_core::catalog::current().terminals.iter().map(|t| t.id.clone()).collect();
                return Err(format!("no terminal called \"{terminal}\" is known; these are: {}", known.join(", ")));
            }
            agent.deliver = Some(Deliver::Keys { terminal, pane });
        }
        (None, None) => {}
        _ => return Err("--terminal and --pane go together: the terminal, and its pane".into()),
    }
    if agent.id.is_empty() {
        return Err("--id is required: any name that stays the same for this agent's session".into());
    }
    if agent.harness.is_empty() {
        return Err(format!("--harness is required: the coding tool the agent runs in, by its id ({}, or your own)", super::names().join(", ")));
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
        let until = within.map(|s| airband_core::clock::now_secs() + s);
        let ask = airband_core::agent::PermissionAsk { schema: SCHEMA.into(), agent, what, detail: String::new(), question: None, until, settled_in: None };
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
        Some("notice") => Event::Notice { text: body()?, leave: false },
        // a picture goes with the next thing the agent says: `show` for one he should see, `picture` for one it only looked at
        Some(word @ ("picture" | "show")) => Event::Picture { path: std::path::absolute(rest.get(1).ok_or("give the picture's file")?).map_err(|e| e.to_string())?.display().to_string(), shown: word == "show" },
        // something it made that lives at a web address, shown with the next thing it says: `made <address> [title]`
        Some("made") => Event::Made { address: rest.get(1).filter(|a| airband_core::links::web(a).is_some()).ok_or("give the web address of what was made")?.clone(), title: rest.get(2).cloned(), kind: None },
        Some("doing") => Event::Activity { text: body()? },
        // what it runs on: `model "<its name>" high`; the effort is left off where the harness has none
        Some("model") => Event::Model { name: rest.get(1).ok_or("give the model's name")?.clone(), effort: rest.get(2).cloned() },
        Some("signoff") => Event::Signoff,
        _ => return Err("expected one of: checkin, working, doing <text>, model <name> [effort], said <text>, asked <text>, notice <text>, show <picture file>, picture <picture file>, made <address> [title], signoff".into()),
    };
    said(post(&AgentEvent::new(agent, event)))
}

/// What `airband report` comes to: done when the tower took the event or it is kept for the tower, so a
/// script that tries again on failure does not send it twice; a failure only when it is lost.
fn said(posted: Posted) -> Result<(), String> {
    match posted {
        Posted::Taken => Ok(()),
        Posted::Kept => {
            eprintln!("airband did not answer; the report is kept and handed over when it is back.");
            Ok(())
        }
        Posted::Refused(why) => Err(format!("airband refused the report: {why}")),
        Posted::Lost => Err("airband did not answer, and the report could not be kept (is it running? `airband status`)".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_kept_for_the_tower_is_done_and_only_a_lost_one_fails() {
        assert!(said(Posted::Taken).is_ok());
        assert!(said(Posted::Kept).is_ok());
        assert!(said(Posted::Lost).is_err());
        assert!(said(Posted::Refused("not an agent".into())).is_err());
    }
}
