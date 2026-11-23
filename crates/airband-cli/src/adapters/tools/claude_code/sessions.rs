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

/// The sessions he has open in a terminal, from the record Claude Code keeps of each while it runs
/// (`sessions/<process>.json` in its folder: the session, its folder, the socket it takes messages on) and
/// the token for that socket beside it. A one-off run (`claude -p`) and a session another program drives
/// have a record too, with another `entrypoint`: those are not his to talk to and are left out.
pub(super) fn open_in(claude: &std::path::Path, running: &dyn Fn(u32, &str) -> bool) -> Option<Vec<crate::adapters::Open>> {
    let dir = claude.join(said("sessions"));
    let read = |path: &std::path::Path| airband_io::files::read_bytes(path).ok().flatten().and_then(|b| first_object(&b));
    // no folder of records at all: Claude Code cannot say what is open
    if !dir.is_dir() {
        return None;
    }
    // what each file is, by what it holds, never by how its name ends: a session's record says its kind, and the
    // token for its socket is the one that holds a peer token
    let files: Vec<(std::path::PathBuf, Value)> = airband_io::files::entries(&dir).ok()?.into_iter().filter_map(|p| read(&p).map(|v| (p, v))).collect();
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
    Some(open)
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
