//! The ways his words get to an agent, by whichever way that agent said it can be reached (`Deliver` in
//! airband-core). A new way to reach an agent is a new variant there and a new arm here. Each arm writes what
//! its variant is defined to take: `Socket` the one line of JSON its doc gives, `Command` the words on standard
//! input, and so on. A tool that takes his words in a way of its own that is none of these goes through its
//! adapter (`Deliver::Harness`).
use super::super::*;
use super::Boxed;

pub(in crate::tower) trait Deliverer: Sync {
    /// Get `words` to this agent. `lines` are the chat lines they are: an agent whose tool writes down the id a
    /// delivery came under has them marked read when it takes them in.
    fn deliver<'a>(&'a self, how: &'a Deliver, agent: &'a str, words: &'a str, lines: &'a [String]) -> Boxed<'a, Result<(), String>>;
}

/// The real ways, from this tower.
pub(in crate::tower) struct AgentWays(pub(in crate::tower) Arc<App>);

/// How long a way other than one the tower holds has to take his words.
const TAKES_WITHIN: Duration = Duration::from_secs(30);

impl Deliverer for AgentWays {
    fn deliver<'a>(&'a self, how: &'a Deliver, agent: &'a str, words: &'a str, lines: &'a [String]) -> Boxed<'a, Result<(), String>> {
        Box::pin(async move {
            let app = &self.0;
            if *how == Deliver::Held {
                // one the tower runs may have to be carried on first, which has bounds of its own, and runs as
                // its own task: cut part way, by a timeout or by whoever asked going away, its holder would go
                // on starting with nobody keeping track of it
                let (app, agent, words, lines) = (app.clone(), agent.to_string(), words.to_string(), lines.to_vec());
                return tokio::spawn(async move { held::tell(&app, &agent, &words, &lines).await }).await.map_err(|e| e.to_string())?;
            }
            tokio::time::timeout(TAKES_WITHIN, other_way(app, how, agent, words, lines))
                .await
                .map_err(|_| format!("it did not take the message within {} seconds", TAKES_WITHIN.as_secs()))?
        })
    }
}

async fn other_way(app: &Arc<App>, how: &Deliver, agent: &str, words: &str, lines: &[String]) -> Result<(), String> {
    match how {
        Deliver::Held => Err("an agent the tower holds is told through its holder".into()),
        Deliver::Socket { path, token } => {
            let mut text = String::new();
            if let Some(t) = token {
                text.push_str(&format!("{}\n", json!({ "type": "auth", "token": t })));
            }
            // the session writes this id on its own record of the turn (`uuid`), which is how it is known read
            let receipt = receipt();
            text.push_str(&format!("{}\n", json!({ "type": "user", "uuid": receipt, "message": { "role": "user", "content": words } })));
            inbox_write(path, text.as_bytes()).await?;
            app.change().receipts.insert(receipt, lines.to_vec());
            Ok(())
        }
        Deliver::Command { argv } => {
            let (program, args) = argv.split_first().ok_or("its delivery command is empty")?;
            let mut child = tokio::process::Command::new(program)
                .args(args)
                .env("AIRBAND_AGENT_ID", agent)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("cannot start {program}: {e}"))?;
            let mut stdin = child.stdin.take().ok_or("it gave no way to write to it")?;
            stdin.write_all(words.as_bytes()).await.map_err(|e| e.to_string())?;
            drop(stdin);
            let status = child.wait().await.map_err(|e| e.to_string())?;
            if status.success() { Ok(()) } else { Err(format!("{program} exited with {status}")) }
        }
        Deliver::Keys { terminal, pane } => {
            let Some(row) = airband_core::catalog::terminal(terminal) else { return Err(format!("no terminal called \"{terminal}\" is known")) };
            // a newline typed into a terminal would send the message early
            let line = words.split_whitespace().collect::<Vec<_>>().join(" ");
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
            let (name, session, words) = (name.clone(), session.clone(), words.to_string());
            tokio::task::spawn_blocking(move || crate::adapters::deliver(&name, &session, &words)).await.map_err(|e| e.to_string())?
        }
        Deliver::Unknown => Err("this tower does not know its kind of delivery".into()),
    }
}

#[cfg(unix)]
pub(in crate::tower) async fn inbox_write(socket: &str, bytes: &[u8]) -> Result<(), String> {
    let mut s = tokio::net::UnixStream::connect(socket).await.map_err(|e| format!("cannot open its inbox: {e}"))?;
    s.write_all(bytes).await.map_err(|e| e.to_string())?;
    s.shutdown().await.map_err(|e| e.to_string())
}
#[cfg(windows)]
pub(in crate::tower) async fn inbox_write(socket: &str, bytes: &[u8]) -> Result<(), String> {
    // not yet run against a real Windows session: this assumes the inbox is a named pipe there
    let mut s = tokio::net::windows::named_pipe::ClientOptions::new().open(socket).map_err(|e| format!("cannot open its inbox: {e}"))?;
    s.write_all(bytes).await.map_err(|e| e.to_string())?;
    s.shutdown().await.map_err(|e| e.to_string())
}
