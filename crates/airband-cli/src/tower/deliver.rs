//! Getting his words to an agent, by whichever way that agent said it can be reached (`Deliver` in
//! airband-core). A new way to reach an agent is a new variant there and a new arm here.
use super::*;

pub(super) async fn deliver(how: &Deliver, agent_id: &str, content: &str) -> Result<(), String> {
    let run = async {
        match how {
            Deliver::ClaudeCodeInbox { socket, token } => {
                let mut lines = String::new();
                if let Some(t) = token {
                    lines.push_str(&format!("{}\n", json!({ "type": "auth", "token": t })));
                }
                lines.push_str(&format!("{}\n", json!({ "type": "user", "message": { "role": "user", "content": content } })));
                inbox_write(socket, lines.as_bytes()).await
            }
            Deliver::Command { argv } => {
                let (program, args) = argv.split_first().ok_or("its delivery command is empty")?;
                let mut child = tokio::process::Command::new(program)
                    .args(args)
                    .env("AIRBAND_AGENT_ID", agent_id)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|e| format!("cannot start {program}: {e}"))?;
                let mut stdin = child.stdin.take().unwrap();
                stdin.write_all(content.as_bytes()).await.map_err(|e| e.to_string())?;
                drop(stdin);
                let status = child.wait().await.map_err(|e| e.to_string())?;
                if status.success() { Ok(()) } else { Err(format!("{program} exited with {status}")) }
            }
            Deliver::Tmux { target } => {
                // a newline typed into a terminal would send the message early
                let line = content.split_whitespace().collect::<Vec<_>>().join(" ");
                for args in [vec!["send-keys", "-t", target, "-l", "--", &line], vec!["send-keys", "-t", target, "Enter"]] {
                    let out = tokio::process::Command::new("tmux").args(&args).output().await.map_err(|e| format!("cannot run tmux: {e}"))?;
                    if !out.status.success() {
                        return Err(format!("tmux: {}", String::from_utf8_lossy(&out.stderr).trim()));
                    }
                }
                Ok(())
            }
            Deliver::Unknown => Err("this tower does not know its kind of delivery".into()),
        }
    };
    tokio::time::timeout(Duration::from_secs(30), run).await.map_err(|_| "it did not take the message within 30 seconds".to_string())?
}

#[cfg(unix)]
pub(super) async fn inbox_write(socket: &str, bytes: &[u8]) -> Result<(), String> {
    let mut s = tokio::net::UnixStream::connect(socket).await.map_err(|e| format!("cannot open its inbox: {e}"))?;
    s.write_all(bytes).await.map_err(|e| e.to_string())?;
    s.shutdown().await.map_err(|e| e.to_string())
}
#[cfg(windows)]
pub(super) async fn inbox_write(socket: &str, bytes: &[u8]) -> Result<(), String> {
    // not yet run against a real Windows session: this assumes the inbox is a named pipe there
    let mut s = tokio::net::windows::named_pipe::ClientOptions::new().open(socket).map_err(|e| format!("cannot open its inbox: {e}"))?;
    s.write_all(bytes).await.map_err(|e| e.to_string())?;
    s.shutdown().await.map_err(|e| e.to_string())
}
