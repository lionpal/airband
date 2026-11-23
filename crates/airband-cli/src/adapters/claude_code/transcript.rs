//! Reading a session's transcript: what it is doing, what it touched, what he typed, the name he gave it, the pull
//! requests it made. Each reader reads only what was written since it last looked, or the end of the file.
use super::*;

/// The name last given to `/remote-control` in this session, read from the whole transcript once and then
/// from what has been written since.
pub(super) fn remote_control_name(transcript: &str, session: &str) -> Option<String> {
    let kept = new_lines(transcript, session, "json", FirstLook::FromStart, |line, kept| {
        if let Some(found) = remote_control_line(line) {
            kept["name"] = found.into();
        }
    })?;
    kept["name"].as_str().map(String::from)
}

/// The kind of link (`catalog/links`) a pull request is.
pub(super) const PULL_KIND: &str = "pull-request";

/// The pull requests this session has opened since the last look. Claude Code writes one line into the
/// session's transcript for each (`pr-link`: its address, its number and the repository), which is what its
/// own screen shows them from. Each is reported once. On the first look at a session nothing is reported: what
/// its transcript already holds was made before anyone was watching.
pub(super) fn pull_requests(hook: &Value) -> Vec<Event> {
    let (Some(transcript), Some(session)) = (hook["transcript_path"].as_str(), hook["session_id"].as_str()) else { return Vec::new() };
    let mut found: Vec<Event> = Vec::new();
    new_lines(transcript, session, "made.json", FirstLook::FromEnd, |line, _| {
        if let Some(made) = pull_request_line(line).filter(|made| !found.contains(made)) {
            found.push(made);
        }
    });
    found
}

/// Where a first look at a session's transcript starts.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum FirstLook {
    /// From its start: everything in it counts.
    FromStart,
    /// From where it ends now: only what is written from now on counts. A session that has written nothing
    /// yet has no transcript, and everything it comes to hold is then new.
    FromEnd,
}

/// Hand each whole line written to a session's transcript since the last look to `each`, with the note kept
/// for this reader (`<session>.<note>` beside the tower's files) to keep what it finds in. A transcript can run
/// to tens of megabytes, so the place reached is kept in the note too and only what follows it is read. A
/// transcript that shrank was replaced, and is read again from its start. Returns the note as it now stands.
pub(super) fn new_lines(transcript: &str, session: &str, note: &str, first: FirstLook, mut each: impl FnMut(&[u8], &mut Value)) -> Option<Value> {
    // the session id names a file of ours: nothing in it may leave the folder
    if !super::file_safe(session) {
        return None;
    }
    let path = super::super::claude_accounts::dir().join(format!("{session}.{note}"));
    // a note that cannot be read is set aside and started again; one that cannot even be moved is left as it is,
    // and this transcript is not read until it can be
    let (note, keeper, loaded) = airband_io::kept::Keeper::open::<serde_json::Map<String, Value>>(Some(path));
    if let Some(trouble) = loaded.trouble() {
        eprintln!("airband: {trouble}");
    }
    if matches!(loaded, airband_io::kept::Loaded::LeftInPlace { .. }) {
        return None;
    }
    let mut kept = Value::Object(note);
    let keep = |kept: &Value| {
        if let Some(why) = keeper.save(kept).news() {
            eprintln!("airband: {why}");
        }
    };
    let Ok(meta) = airband_io::files::metadata(std::path::Path::new(transcript)) else {
        if first == FirstLook::FromEnd && !std::path::Path::new(transcript).exists() && kept.get("at").is_none() {
            kept["at"] = 0.into();
            keep(&kept);
        }
        return Some(kept);
    };
    let len = meta.len();
    let at = match kept["at"].as_u64() {
        Some(at) if at <= len => at,
        Some(_) => 0,
        None if first == FirstLook::FromEnd => len,
        None => 0,
    };
    let reached = whole_lines(std::path::Path::new(transcript), at, |line| each(line, &mut kept)).unwrap_or(at);
    if kept["at"].as_u64() != Some(reached) || reached > at {
        kept["at"] = reached.into();
        keep(&kept);
    }
    Some(kept)
}

/// The pull request one transcript line records, when the line is Claude Code's own record of one. Such a
/// record is a short line; the long ones (a request, a tool's output) are not read at all.
pub(super) fn pull_request_line(line: &[u8]) -> Option<Event> {
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
pub(super) fn whole_lines(path: &std::path::Path, from: u64, mut each: impl FnMut(&[u8])) -> Option<u64> {
    // read a part at a time, so a long transcript costs the memory of one part and one line
    const PART: u64 = 1024 * 1024;
    let (mut at, mut read_to, mut line) = (from, from, Vec::new());
    loop {
        let part = match airband_io::files::read_part(path, read_to, PART) {
            Ok(part) => part,
            Err(_) if read_to == from => return None,
            Err(_) => return Some(at),
        };
        if part.is_empty() {
            return Some(at);
        }
        read_to += part.len() as u64;
        let mut rest = part.as_slice();
        while let Some(end) = rest.iter().position(|b| *b == b'\n') {
            line.extend_from_slice(&rest[..=end]);
            at += line.len() as u64;
            each(&line);
            line.clear();
            rest = &rest[end + 1..];
        }
        line.extend_from_slice(rest);
    }
}

/// The name in one transcript line, if that line is Claude Code's own record of him running
/// `/remote-control <name>`: an entry of its own kind with the command and what he gave it as fields
/// (`commandRun`). Nothing is read out of a line's words, so one that only quotes such a command is not one.
pub(super) fn remote_control_line(line: &[u8]) -> Option<String> {
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

/// A look at a session, from the end of its transcript: what it is doing right now, in its own words for its
/// latest step, and the model and effort its latest answer was made with. A command or a subagent comes with a
/// description Claude wrote for it ("Run the payments tests"); for the rest, a few plain words. Reads at most
/// the last 96 KB of the file, so it is quick however long the session.
pub(crate) fn glance(agent: &Agent) -> super::super::Glance {
    let Some(tail) = tail_of(agent, 96 * 1024) else { return super::super::Glance::default() };
    let (model, effort) = engine_in(&tail).unzip();
    super::super::Glance { doing: step_in(&tail), model, effort: effort.flatten() }
}

/// The end of a session's transcript: at most this many bytes of it.
pub(super) fn tail_of(agent: &Agent, most: u64) -> Option<String> {
    let path = std::path::Path::new(agent.meta.get("transcript")?.as_str()?);
    let size = airband_io::files::metadata(path).ok()?.len();
    let tail = airband_io::files::read_part(path, size.saturating_sub(most), most).ok()?;
    Some(String::from_utf8_lossy(&tail).into_owned())
}

/// What he has typed into a session since the byte `from` of its transcript, and the byte to read from next
/// time. The first time, at most the last 24 MB are gone through: a line with a pasted picture in it runs to
/// hundreds of kilobytes. The file is read a line at a time and only his own lines are kept, so this costs the
/// memory of one line however much there is to go through. Whole lines only: one still being written is left
/// for the next call.
pub(crate) fn told(agent: &Agent, from: u64) -> (Vec<super::super::Told>, u64) {
    let read = || {
        let path = std::path::Path::new(agent.meta.get("transcript")?.as_str()?);
        let size = airband_io::files::metadata(path).ok()?.len();
        // the transcript was replaced by a shorter one: start over; what was already taken is known by its id
        let from = if from > size { 0 } else { from };
        let fresh = from == 0 && size > 24 * 1024 * 1024;
        let start = if fresh { size - 24 * 1024 * 1024 } else { from };
        let mut told = Vec::new();
        // started in the middle of the file: what is left of the line it landed in is not a line
        let mut skip = fresh;
        let at = whole_lines(path, start, |line| {
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
pub(super) fn told_in(piece: &str) -> Vec<super::super::Told> {
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
                    parts.iter().filter(|p| p["type"] == "image" && p["source"]["type"] == "base64").filter_map(|p| airband_core::encode::unbase64(p["source"]["data"].as_str()?)).take(4).collect()
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
            let at = from["timestamp"].as_str().or(entry["timestamp"].as_str()).and_then(airband_core::clock::parse_rfc3339)?;
            Some(super::super::Told { id: id.to_string(), at, text, images: if his { images } else { Vec::new() }, his })
        })
        .collect()
}

/// The files a session has lately read or changed and the commands it has run, newest first, as its tool's row
/// says each of its tools names what it touches (`touches`).
pub(crate) fn touched(agent: &Agent) -> Vec<String> {
    tail_of(agent, 256 * 1024).map(|tail| touched_in(&tail)).unwrap_or_default()
}

/// Every tool call in a piece of a transcript, oldest first: when its line was written, the tool's name and
/// what it was given. A subagent's calls are passed over: they are its own, not the session's.
pub(super) fn tool_uses(tail: &str) -> Vec<(Option<u64>, String, Value)> {
    let mut calls = Vec::new();
    // a quick look before parsing: most lines hold no call
    for line in tail.lines().filter(|l| l.contains("\"tool_use\"")) {
        let Ok(mut entry) = serde_json::from_str::<Value>(line) else { continue };
        if entry["isSidechain"] == true {
            continue;
        }
        let at = entry["timestamp"].as_str().and_then(airband_core::clock::parse_rfc3339);
        let Some(parts) = entry["message"]["content"].as_array_mut() else { continue };
        for part in parts.iter_mut().filter(|c| c["type"] == "tool_use") {
            if let Some(name) = part["name"].as_str().map(String::from) {
                calls.push((at, name, part["input"].take()));
            }
        }
    }
    calls
}

pub(super) fn touched_in(tail: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for (_, name, input) in tool_uses(tail).iter().rev() {
        // what each of its tools touches is the row's to say: one it says nothing of touches nothing
        let what = super::super::hooks::fill(&super::super::hooks::tool_use(HARNESS, name)["touches"], input).map(|t| airband_core::prose::clip(t.trim(), 160));
        if let Some(what) = what.filter(|w| !w.is_empty() && !found.contains(w)) {
            found.push(what);
        }
        if found.len() >= 14 {
            break;
        }
    }
    found
}

/// The model and effort of the latest answer in a piece of a transcript. A subagent's lines are passed over: it
/// may run on another model.
pub(super) fn engine_in(tail: &str) -> Option<(String, Option<String>)> {
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
pub(super) fn model_name(id: &str) -> Option<String> {
    let row = super::super::tool_row(HARNESS)?;
    if row.with.get("not_models").and_then(Value::as_array).is_some_and(|none| none.iter().any(|n| n.as_str() == Some(id))) {
        return None;
    }
    Some(row.with.get("model_names").and_then(|names| names.get(id)).and_then(Value::as_str).unwrap_or(id).to_string())
}

/// The latest step named in a piece of a transcript.
pub(super) fn step_in(tail: &str) -> Option<String> {
    tool_uses(tail).iter().rev().find_map(|(_, name, input)| step(name, input))
}

/// Every step a session has taken since `since`, oldest first. From the end of its transcript only: a turn
/// longer than that reaches back is told by its later steps.
pub(crate) fn steps(agent: &Agent, since: u64) -> Vec<String> {
    tail_of(agent, 1024 * 1024).map(|tail| steps_in(&tail, since)).unwrap_or_default()
}

pub(super) fn steps_in(tail: &str, since: u64) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for (_, name, input) in tool_uses(tail).iter().filter(|(at, _, _)| at.is_some_and(|at| at >= since)) {
        if let Some(label) = step(name, input) {
            // the same label twice running is one step said twice
            if found.last() != Some(&label) {
                found.push(label);
            }
        }
    }
    let most = 80;
    if found.len() > most { found.split_off(found.len() - most) } else { found }
}

pub(super) fn step(tool: &str, input: &Value) -> Option<String> {
    let text = |k: &str| input[k].as_str().map(str::trim).filter(|t| !t.is_empty());
    // what the row says of this tool of Claude Code's own
    let row = super::super::hooks::tool_use(HARNESS, tool);
    let said = if row["bookkeeping"].as_bool() == Some(true) {
        // bookkeeping of its own, not a step in the work: keep looking further back
        return None;
    } else if let Some(said) = text("description") {
        said.to_string()
    } else if !row.is_null() {
        super::super::hooks::fill(&row["doing"], input)?
    } else {
        // a tool from a server, named by Claude Code as server and tool with its row's mark between: the
        // tool's own name, as words ("take screenshot")
        let mark = super::super::row_says(HARNESS, "server_tool");
        let own = if mark.is_empty() { tool } else { tool.rsplit(mark.as_str()).next().unwrap_or(tool) };
        own.replace(['_', '-'], " ")
    };
    let said: String = said.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(airband_core::prose::clip(&said, 64)).filter(|s| !s.is_empty())
}
