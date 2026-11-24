//! The sessions Claude Code keeps a record of: which are open now, where their transcripts are, and the folders
//! it has worked in.
use super::*;

/// Is the process a session's record names still that session? Its number alone could have been given to
/// another program since; the moment it started could not.
/// The record's start time is the system's own form (`ps -o lstart=` in English at UTC), read as a moment: one
/// that does not read is no match.
pub(super) fn running(pid: u32, started: &str) -> bool {
    #[cfg(unix)]
    {
        crate::platform::lstart(started).is_some_and(|at| crate::platform::started(pid) == Some(at))
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, started);
        false
    }
}

/// What Claude Code's row says, under `what`.
fn said(what: &str) -> String {
    crate::adapters::row_says(super::HARNESS, what)
}

/// The record Claude Code keeps of one running session, by its process: `sessions/<process>.json` in its folder.
pub(super) fn record(claude: &std::path::Path, pid: &str) -> Option<Value> {
    if !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    airband_io::files::read_bytes(&claude.join(said("sessions")).join(format!("{pid}.json"))).ok().flatten().and_then(|b| first_object(&b))
}

/// A session record or key file as Claude Code left it: the one JSON object it starts with. Claude Code rewrites
/// these in place, and a rewrite shorter than the one before can leave the old file's last bytes after the object
/// (seen: one stray `}`). Those bytes are no part of the record; a record that is not one whole object is no record.
fn first_object(bytes: &[u8]) -> Option<Value> {
    serde_json::Deserializer::from_slice(bytes).into_iter::<Value>().next()?.ok().filter(Value::is_object)
}

/// What a file in Claude Code's folder of session records holds, as JSON reads it.
enum Held {
    /// A whole object: a record, or the token for a session's socket.
    Object(Value),
    /// Not yet whole (the start of a value and no more, or nothing yet), or it could not be read: it may be a
    /// record being written.
    Unfinished,
    /// No JSON object at all, or gone: no record of Claude Code's.
    Not,
}

fn held(path: &std::path::Path) -> Held {
    let bytes = match airband_io::files::read_bytes(path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Held::Not,
        Err(_) => return Held::Unfinished,
    };
    match serde_json::Deserializer::from_slice(&bytes).into_iter::<Value>().next() {
        None => Held::Unfinished,
        Some(Ok(v)) if v.is_object() => Held::Object(v),
        Some(Ok(_)) => Held::Not,
        Some(Err(e)) if e.classify() == serde_json::error::Category::Eof => Held::Unfinished,
        Some(Err(_)) => Held::Not,
    }
}

/// The sessions he has open in a terminal, from the record Claude Code keeps of each while it runs
/// (`sessions/<process>.json` in its folder: the session, its folder, the socket it takes messages on) and
/// the token for that socket beside it. A one-off run (`claude -p`) and a session another program drives
/// have a record too, with another `entrypoint`: those are not his to talk to and are left out. A file that
/// holds the start of a JSON value and no more (Claude Code was writing it just then), or that could not be
/// read, makes the listing less than whole: no session is taken to have ended on that look. A file that is no
/// JSON at all (the system's own, an editor's) is no record of Claude Code's, and is passed over.
pub(super) fn open_in(claude: &std::path::Path, running: &dyn Fn(u32, &str) -> bool) -> Option<crate::adapters::Listing> {
    let dir = claude.join(said("sessions"));
    // no folder of records at all: Claude Code cannot say what is open
    if !dir.is_dir() {
        return None;
    }
    // what each file is, by what it holds, never by how its name ends: a session's record says its kind, and the
    // token for its socket is the one that holds a peer token
    let read: Vec<(std::path::PathBuf, Held)> = airband_io::files::entries(&dir).ok()?.into_iter().map(|p| { let v = held(&p); (p, v) }).collect();
    let whole = !read.iter().any(|(_, v)| matches!(v, Held::Unfinished));
    let files: Vec<(std::path::PathBuf, Value)> = read.into_iter().filter_map(|(p, v)| match v { Held::Object(v) => Some((p, v)), _ => None }).collect();
    let mut open = Vec::new();
    for (_, v) in &files {
        let field = |k: &str| text(v, k);
        if v["kind"].as_str() != Some(said("session_kind").as_str()) || v["entrypoint"].as_str() != Some(said("session_entry").as_str()) {
            continue;
        }
        let (Some(pid), Some(id), Some(socket), Some(started)) = (v["pid"].as_u64().map(|p| p as u32), field("sessionId"), field("messagingSocketPath"), field("procStart")) else { continue };
        if !running(pid, &started) {
            continue;
        }
        // the token its socket asks for: in a file of that process's, written when the same process started
        let key = files
            .iter()
            .filter(|(p, k)| k["peerToken"].is_string() && p.file_name().is_some_and(|n| n.to_string_lossy().split('.').next() == Some(pid.to_string().as_str())))
            .find(|(_, k)| k["procStart"].as_str().and_then(crate::platform::lstart).is_some_and(|at| crate::platform::lstart(&started) == Some(at)))
            .map(|(_, k)| k);
        let token = key.and_then(|k| k["peerToken"].as_str().filter(|t| !t.is_empty()).map(String::from));
        let transcript = transcript_of(claude, &id);
        open.push(crate::adapters::Open {
            agent: Agent {
                harness: HARNESS.into(),
                cwd: field("cwd"),
                name: None,
                title: named_by_him(v),
                pid: Some(pid),
                deliver: Some(Deliver::Socket { path: socket, token }),
                meta: transcript.map(|t| serde_json::Map::from_iter([("transcript".to_string(), Value::String(t))])).unwrap_or_default(),
                id,
            },
            idle: v["status"] == "idle",
        });
    }
    Some(crate::adapters::Listing { open, whole })
}

/// Where a session's transcript is: the file named for the session, in one of the folders Claude Code keeps
/// per project.
pub(super) fn transcript_of(claude: &std::path::Path, session: &str) -> Option<String> {
    if !super::file_safe(session) {
        return None;
    }
    airband_io::files::entries(&claude.join(said("projects"))).ok()?.into_iter().map(|d| d.join(format!("{session}.jsonl"))).find(|p| p.is_file()).map(|p| p.display().to_string())
}

/// The folders Claude Code has had sessions in: it keeps a folder of transcripts for each, and a transcript
/// says on its lines which folder the session was in. The newest transcript of each is asked, and only its
/// first lines are read.
pub(super) fn worked_in(claude: &std::path::Path) -> Vec<crate::adapters::Worked> {
    let Ok(projects) = airband_io::files::entries(&claude.join(said("projects"))) else { return Vec::new() };
    let found = projects.into_iter().filter_map(|project| {
        // newest first by when each was last written, read as a time: one whose time cannot be read is not it.
        // A transcript is known by what its lines say (the folder the session was in), never by its name's ending
        let mut files: Vec<(u64, std::path::PathBuf)> = airband_io::files::entries(&project).ok()?.into_iter().filter_map(|p| Some((crate::adapters::written(&p)?, p))).collect();
        files.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        files.into_iter().find_map(|(at, file)| {
            let lines = airband_io::files::first_lines(&file, 40).ok()?;
            let folder = lines.iter().find_map(|line| serde_json::from_str::<Value>(line).ok()?["cwd"].as_str().filter(|c| !c.is_empty()).map(String::from))?;
            Some(crate::adapters::Worked { folder, at })
        })
    });
    crate::adapters::newest(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_is_no_json_at_all_leaves_the_listing_whole_and_one_being_written_does_not() {
        let claude = std::env::temp_dir().join(format!("airband-claude-{}", crate::config::random_hex(6)));
        let sessions = claude.join(said("sessions"));
        std::fs::create_dir_all(&sessions).unwrap();
        let never = |_: u32, _: &str| false;
        // the system's own file, and an editor's: no record of Claude Code's
        std::fs::write(sessions.join(".DS_Store"), [0u8, 0, 0, 1, b'B', b'u', b'd', b'1']).unwrap();
        std::fs::write(sessions.join("notes.txt"), "not a record").unwrap();
        std::fs::write(sessions.join("list.json"), "[1, 2]").unwrap();
        assert!(open_in(&claude, &never).unwrap().whole);
        // a record Claude Code is writing just then: the start of an object, or nothing yet
        std::fs::write(sessions.join("21.json"), r#"{"pid": 21, "sessionId": "s"#).unwrap();
        assert!(!open_in(&claude, &never).unwrap().whole);
        std::fs::write(sessions.join("21.json"), "").unwrap();
        assert!(!open_in(&claude, &never).unwrap().whole);
        let _ = std::fs::remove_dir_all(&claude);
    }
}
