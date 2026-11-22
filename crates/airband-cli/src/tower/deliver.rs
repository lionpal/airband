//! Getting his words to an agent, by whichever way that agent said it can be reached (`Deliver` in
//! airband-core). A new way to reach an agent is a new variant there and a new arm here; a new tool that
//! takes his words in a way of its own is none of that, only its adapter (`Deliver::Harness`).
use super::*;

/// A new id for one delivery, in the form a tool's own records keep ids in.
pub(super) fn receipt() -> String {
    let h = config::random_hex(16);
    format!("{}-{}-4{}-8{}-{}", &h[..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}

/// `lines` are the chat lines these words are: an agent whose tool writes down the id a delivery came under
/// has them marked read when it takes them in.
pub(super) async fn deliver(app: &Arc<App>, how: &Deliver, agent_id: &str, content: &str, lines: &[String]) -> Result<(), String> {
    let run = async {
        match how {
            Deliver::Held => held::tell(app, agent_id, content, lines).await,
            Deliver::Socket { path, token } => {
                let mut text = String::new();
                if let Some(t) = token {
                    text.push_str(&format!("{}\n", json!({ "type": "auth", "token": t })));
                }
                // the session writes this id on its own record of the turn (`uuid`), which is how it is known read
                let receipt = receipt();
                text.push_str(&format!("{}\n", json!({ "type": "user", "uuid": receipt, "message": { "role": "user", "content": content } })));
                inbox_write(path, text.as_bytes()).await?;
                app.passing().receipts.insert(receipt, lines.to_vec());
                Ok(())
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
            Deliver::Keys { terminal, pane } => {
                let Some(row) = airband_core::catalog::terminal(terminal) else { return Err(format!("no terminal called \"{terminal}\" is known")) };
                // a newline typed into a terminal would send the message early
                let line = content.split_whitespace().collect::<Vec<_>>().join(" ");
                for args in row.runs(pane, &line) {
                    let out = tokio::process::Command::new(&row.program).args(&args).output().await.map_err(|e| format!("cannot run {}: {e}", row.program))?;
                    if !out.status.success() {
                        return Err(format!("{}: {}", row.program, String::from_utf8_lossy(&out.stderr).trim()));
                    }
                }
                Ok(())
            }
            // by the tool's own way into a session that is already open, which its adapter knows
            Deliver::Harness { name, session } => {
                let (name, session, words) = (name.clone(), session.clone(), content.to_string());
                tokio::task::spawn_blocking(move || crate::adapters::deliver(&name, &session, &words)).await.map_err(|e| e.to_string())?
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
