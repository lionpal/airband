//! Agents the tower runs itself. An agent he starts in a terminal belongs to that terminal: the tower can only
//! leave it a message from outside, which the agent treats as such. One started here is a process whose input
//! the tower holds, so what he says is written there as his own turn, with nothing around it, and waits its
//! turn in the agent's own queue.
//!
//! Nothing here knows which agent tool it is. The launcher in the configuration says how to start a program
//! that reads one message a line (`argv`), how to begin a new session or carry one on (`held.fresh`,
//! `held.resume`), and what a line of input looks like (`held.line`). The agent reports through its harness's
//! adapter like any other; its output on this pipe is read only so the pipe does not fill.
//!
//! Each runs in a holder of its own (`airband hold`), a process apart from the tower that keeps the program
//! and its input open and listens on a socket. The tower writes there. So an agent goes on working while the
//! tower is restarted or updated; the tower that starts finds the socket again. Every one is also remembered,
//! so one whose holder has gone (the computer was restarted) is carried on from where its session was.
use super::*;

/// One held agent, as remembered across restarts.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Kept {
    /// Its session's id, which is also its agent id.
    pub(super) id: String,
    pub(super) cwd: String,
    /// The launcher that started it, by name, and the options it was started with (model, effort).
    pub(super) launcher: String,
    #[serde(default)]
    pub(super) options: Vec<String>,
}

/// Where a held agent's holder listens: a short path (a socket's may not be long), of this user's own, that is
/// the same for the same agent of the same tower every time.
fn socket(id: &str) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let name: String = Sha256::digest(format!("{}|{id}", config::dir().display()).as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect();
    #[cfg(unix)]
    let user = unsafe { libc::getuid() };
    #[cfg(not(unix))]
    let user = 0;
    std::path::PathBuf::from(format!("/tmp/airband-{user}")).join(format!("h-{name}.sock"))
}

/// Write to a held agent's holder. Fails when there is none listening: it has ended, or was never started.
#[cfg(unix)]
async fn write(id: &str, bytes: &[u8]) -> Result<(), String> {
    let mut to = tokio::net::UnixStream::connect(socket(id)).await.map_err(|e| format!("it is not running: {e}"))?;
    to.write_all(bytes).await.map_err(|e| e.to_string())?;
    to.shutdown().await.map_err(|e| e.to_string())
}
#[cfg(not(unix))]
async fn write(_: &str, _: &[u8]) -> Result<(), String> {
    Err("the tower cannot run agents itself on this system yet".into())
}

/// A session id of the usual shape, made here so the agent is known by it before it has said a word.
pub(super) fn new_id() -> String {
    let h = config::random_hex(16);
    format!("{}-{}-4{}-a{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}

/// Start the program for `kept`, new or carried on, in a holder of its own (`airband hold`): a process apart
/// from the tower, so the agent goes on when the tower is restarted.
#[cfg(unix)]
async fn spawn(app: &Arc<App>, kept: &Kept, fresh: bool) -> Result<(), String> {
    let launcher = app.cfg.launchers.get(&kept.launcher).ok_or_else(|| format!("no launcher called {} is configured", kept.launcher))?;
    let held = launcher.held.as_ref().ok_or_else(|| format!("the launcher {} does not hold its agents", kept.launcher))?;
    let session: Vec<String> = if fresh { &held.fresh } else { &held.resume }.iter().map(|a| a.replace("{id}", &kept.id)).collect();
    let argv: Vec<String> = launcher.argv.iter().flat_map(|a| if a == "{options}" { kept.options.clone() } else { vec![a.replace("{cwd}", &kept.cwd).replace("{id}", &kept.id)] }).chain(session).collect();
    let sock = socket(&kept.id);
    if let Some(dir) = sock.parent() {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut holder = std::process::Command::new(me);
    holder
        .arg("hold")
        .arg("--sock")
        .arg(&sock)
        .arg("--cwd")
        .arg(&kept.cwd)
        .arg("--")
        .args(&argv)
        // AIRBAND_INTERNAL must not leak in: the agent has to report to the tower. AIRBAND_HELD tells its
        // adapter that this tower holds it, so it says so when it reports in.
        .env_remove("AIRBAND_INTERNAL")
        .env("AIRBAND_HELD", "1")
        .env("AIRBAND_HELD_ID", &kept.id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // a session of its own: whatever stops the tower and its children leaves this alone
    unsafe {
        use std::os::unix::process::CommandExt;
        holder.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let _ = std::fs::remove_file(&sock);
    let mut child = holder.spawn().map_err(|e| format!("its holder did not start: {e}"))?;
    // it is not waited for as a child would be: collect it when it ends, so it does not linger as a dead entry
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    for _ in 0..50 {
        if tokio::net::UnixStream::connect(&sock).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!("{} did not start", argv.first().cloned().unwrap_or_default()))
}
#[cfg(not(unix))]
async fn spawn(_: &Arc<App>, _: &Kept, _: bool) -> Result<(), String> {
    Err("the tower cannot run agents itself on this system yet".into())
}

/// Write one message of his to a held agent as its next turn. One whose process has gone (the tower restarted,
/// or it crashed) is carried on first.
pub(super) async fn tell(app: &Arc<App>, id: &str, text: &str) -> Result<(), String> {
    let kept = app.inner.lock().unwrap().held.iter().find(|k| k.id == id).cloned().ok_or("the tower no longer holds this agent")?;
    let held = app.cfg.launchers.get(&kept.launcher).and_then(|l| l.held.clone()).ok_or("its launcher is no longer configured")?;
    let line = format!("{}\n", held.line.replace("{text}", &Value::String(text.to_string()).to_string()));
    if write(id, line.as_bytes()).await.is_ok() {
        return Ok(());
    }
    // its holder has gone (the computer restarted, or it crashed): carry the session on, then say it
    spawn(app, &kept, false).await?;
    write(id, line.as_bytes()).await
}

/// A held agent has stopped (it finished its turn, or he stopped it): everything he said to it meanwhile, and
/// did not take back, goes to it now as one turn, in the order he said it.
pub(super) async fn flush(app: &Arc<App>, id: &str) {
    let waiting: Vec<(String, String)> = {
        let mut inner = app.inner.lock().unwrap();
        let Some(s) = inner.session(id).filter(|s| s.waiting && !s.queue.is_empty()) else { return };
        s.waiting = false;
        let waiting = std::mem::take(&mut s.queue);
        inner.drop_pending(id, None);
        waiting
    };
    let words = waiting.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>().join("\n\n");
    let told = tell(app, id, &words).await;
    let mut inner = app.inner.lock().unwrap();
    match told {
        Ok(()) => {
            for (line, _) in &waiting {
                if let Some(e) = inner.log.iter_mut().find(|e| e.id == *line) {
                    e.queued = false;
                }
                inner.touch(line);
            }
        }
        Err(e) => {
            // not lost: they wait for its next stop
            log(format!("could not pass on what was waiting for a held agent: {e}"));
            if let Some(s) = inner.session(id) {
                s.waiting = true;
                s.queue.splice(0..0, waiting);
            }
        }
    }
    app.save(&inner);
    app.kick.notify_one();
}

/// Stop what a held agent is doing, now. What he said to it meanwhile is then read together as its next turn.
pub(super) async fn stop(app: &Arc<App>, id: &str) -> Result<(), String> {
    let (session, line) = {
        let inner = app.inner.lock().unwrap();
        let s = inner.find(id).cloned().ok_or("That agent is no longer running.")?;
        if s.agent.deliver != Some(Deliver::Held) {
            return Err("Only an agent airband runs can be stopped from here. Take it over first.".into());
        }
        let line = inner.held.iter().find(|k| k.id == id).and_then(|k| app.cfg.launchers.get(&k.launcher)).and_then(|l| l.held.as_ref()).and_then(|h| h.stop.clone());
        (s, line.ok_or("Its launcher has no way to stop it.")?)
    };
    if session.waiting {
        return Err("It is not doing anything.".into());
    }
    write(id, format!("{}\n", line.replace("{id}", &config::random_hex(8))).as_bytes()).await.map_err(|_| "Its process has gone.".to_string())?;
    log(format!("stopped {}", session.callsign.text()));
    // a harness may or may not report a stopped turn as finished: either way it is waiting from now
    {
        let mut inner = app.inner.lock().unwrap();
        if let Some(s) = inner.session(id) {
            s.waiting = true;
            s.doing = None;
            s.subagents = 0;
        }
        app.save(&inner);
    }
    app.kick.notify_one();
    // let it settle before it is given more
    tokio::time::sleep(Duration::from_millis(400)).await;
    flush(app, id).await;
    Ok(())
}

/// He took back something he said that had not been delivered yet.
pub(super) fn unsend(app: &App, line: &str) -> Result<(), String> {
    let mut inner = app.inner.lock().unwrap();
    let mut found = false;
    for s in inner.sessions.iter_mut() {
        let before = s.queue.len();
        s.queue.retain(|(id, _)| id != line);
        found |= s.queue.len() != before;
    }
    if !found {
        return Err("It has already been delivered.".into());
    }
    if let Some(e) = inner.log.iter_mut().find(|e| e.id == line) {
        e.queued = false;
        e.cancelled = true;
    }
    inner.touch(line);
    app.save(&inner);
    app.kick.notify_one();
    Ok(())
}

/// Start a new held agent in `cwd` and give it its task.
pub(super) async fn start(app: &Arc<App>, launcher: &str, cwd: &str, options: Vec<String>, task: &str) -> Result<String, String> {
    let kept = Kept { id: new_id(), cwd: cwd.to_string(), launcher: launcher.to_string(), options };
    spawn(app, &kept, true).await?;
    {
        let mut inner = app.inner.lock().unwrap();
        inner.held.push(kept.clone());
        app.save(&inner);
    }
    tell(app, &kept.id, task).await?;
    Ok(kept.id)
}

/// The tower has started: carry on every agent it was holding, so each is there to be spoken to. One whose
/// session cannot be carried on is let go.
pub(super) async fn carry_on(app: &Arc<App>) {
    let kept: Vec<Kept> = app.inner.lock().unwrap().held.clone();
    for k in kept {
        // only the ones still on the radio: one that signed off is finished
        if app.inner.lock().unwrap().find(&k.id).is_none() {
            continue;
        }
        // its holder outlived the last tower: it is still running, and there is nothing to do
        if write(&k.id, b"").await.is_ok() {
            continue;
        }
        if let Err(e) = spawn(app, &k, false).await {
            log(format!("could not carry on held agent {}: {e}", &k.id[..8.min(k.id.len())]));
        }
    }
}

/// Let a held agent go: its input is closed, and it ends by itself and signs off.
pub(super) async fn end(app: &Arc<App>, id: &str) -> bool {
    #[cfg(unix)]
    let was = write(id, crate::hold::END).await.is_ok();
    #[cfg(not(unix))]
    let was = false;
    let mut inner = app.inner.lock().unwrap();
    let kept = inner.held.len();
    inner.held.retain(|k| k.id != id);
    if inner.held.len() != kept {
        app.save(&inner);
    }
    was || inner.held.len() != kept
}

/// An agent has signed off: if the tower held it, it holds it no more.
pub(super) fn forget(inner: &mut Inner, id: &str) {
    inner.held.retain(|k| k.id != id);
}

/// Take over an agent he started in a terminal of his own: end that process and carry the same session on in
/// one the tower holds. Only for an agent whose process the tower knows, so that two never run one session.
pub(super) async fn take_over(app: &Arc<App>, id: &str) -> Result<(), String> {
    let (session, launcher) = {
        let inner = app.inner.lock().unwrap();
        let s = inner.find(id).cloned().ok_or("That agent is no longer running.")?;
        // carried on by a launcher that runs this agent's own harness: never one tool's session under another
        let launcher = app.cfg.launcher_for(Some(&s.agent.harness)).filter(|(_, l)| l.held.is_some()).map(|(name, _)| name.clone());
        (s, launcher)
    };
    if session.agent.deliver == Some(Deliver::Held) {
        return Err("The tower already runs this one.".into());
    }
    let Some(launcher) = launcher else {
        return Err(format!("No launcher here runs {} agents itself.", session.agent.harness));
    };
    let cwd = session.agent.cwd.clone().ok_or("The tower does not know its folder.")?;
    let pid = session.agent.pid.ok_or("The tower does not know its process. Close it in its terminal, then start it from here.")?;
    if !session.waiting {
        return Err("It is in the middle of something. Take it over when it has stopped.".into());
    }
    // ask it to end, and wait for it to be gone: two processes must never run one session
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
    for _ in 0..50 {
        if !alive(pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    if alive(pid) {
        return Err("Its terminal session did not end. Close it there, then try again.".into());
    }
    let kept = Kept { id: id.to_string(), cwd, launcher, options: Vec::new() };
    // ending it signed it off; it reports in again from the new process, under the same id
    spawn(app, &kept, false).await?;
    let mut inner = app.inner.lock().unwrap();
    inner.held.retain(|k| k.id != id);
    inner.held.push(kept);
    // it is on the radio from now, before it has reported in: held, and reachable
    let mut agent = session.agent.clone();
    agent.deliver = Some(Deliver::Held);
    agent.pid = None;
    inner.check_in(&agent);
    if let Some(s) = inner.session(id) {
        s.agent.deliver = Some(Deliver::Held);
        s.agent.pid = None;
        s.callsign = session.callsign.clone();
        s.waiting = true;
    }
    app.save(&inner);
    app.kick.notify_one();
    log(format!("took over {}", session.callsign.text()));
    Ok(())
}
