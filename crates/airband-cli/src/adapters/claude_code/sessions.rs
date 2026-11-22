//! The sessions Claude Code keeps a record of: which are open now, where their transcripts are, and the folders
//! it has worked in.
#![allow(clippy::disallowed_methods, reason = "reads the tool's own session files")]
use super::*;

/// Is the process a session's record names still that session? Its number alone could have been given to
/// another program since; the moment it started could not.
pub(super) fn running(pid: u32, started: &str) -> bool {
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

/// The record Claude Code keeps of one running session, by its process: `sessions/<process>.json` in its folder.
pub(super) fn record(claude: &std::path::Path, pid: &str) -> Option<Value> {
    if !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    std::fs::read(claude.join("sessions").join(format!("{pid}.json"))).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// The sessions he has open in a terminal, from the record Claude Code keeps of each while it runs
/// (`sessions/<process>.json` in its folder: the session, its folder, the socket it takes messages on) and
/// the token for that socket beside it. A one-off run (`claude -p`) and a session another program drives
/// have a record too, with another `entrypoint`: those are not his to talk to and are left out.
pub(super) fn open_in(claude: &std::path::Path, running: &dyn Fn(u32, &str) -> bool) -> Option<Vec<super::super::Open>> {
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
        open.push(super::super::Open {
            agent: Agent {
                harness: HARNESS.into(),
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
pub(super) fn transcript_of(claude: &std::path::Path, session: &str) -> Option<String> {
    if !super::file_safe(session) {
        return None;
    }
    std::fs::read_dir(claude.join("projects")).ok()?.flatten().map(|d| d.path().join(format!("{session}.jsonl"))).find(|p| p.is_file()).map(|p| p.display().to_string())
}

/// The folders Claude Code has had sessions in: it keeps a folder of transcripts for each, and a transcript
/// says on its lines which folder the session was in. The newest transcript of each is asked, and only its
/// first lines are read.
pub(super) fn worked_in(claude: &std::path::Path) -> Vec<super::super::Worked> {
    use std::io::BufRead;
    let Ok(projects) = std::fs::read_dir(claude.join("projects")) else { return Vec::new() };
    let found = projects.flatten().filter_map(|project| {
        let newest = std::fs::read_dir(project.path()).ok()?.flatten().map(|f| f.path()).filter(|p| p.extension().is_some_and(|e| e == "jsonl")).map(|p| (super::super::written(&p), p)).max_by_key(|(at, _)| *at)?;
        let lines = std::io::BufReader::new(std::fs::File::open(&newest.1).ok()?).lines();
        let folder = lines.take(40).map_while(Result::ok).find_map(|line| serde_json::from_str::<Value>(&line).ok()?["cwd"].as_str().filter(|c| !c.is_empty()).map(String::from))?;
        Some(super::super::Worked { folder, at: newest.0 })
    });
    super::super::newest(found)
}
