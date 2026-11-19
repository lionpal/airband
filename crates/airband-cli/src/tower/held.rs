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
//! A held agent lives as long as the tower: when the tower stops, the agent's input closes and it exits. So
//! every one is remembered, and a tower that starts carries each on from where its session was.
use super::*;
use std::sync::OnceLock;
use tokio::io::AsyncReadExt;
use tokio::process::ChildStdin;

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

struct Running {
    input: tokio::sync::Mutex<ChildStdin>,
    pid: Option<u32>,
}

static RUNNING: OnceLock<Mutex<HashMap<String, Arc<Running>>>> = OnceLock::new();
fn running() -> &'static Mutex<HashMap<String, Arc<Running>>> {
    RUNNING.get_or_init(Default::default)
}

/// A session id of the usual shape, made here so the agent is known by it before it has said a word.
pub(super) fn new_id() -> String {
    let h = config::random_hex(16);
    format!("{}-{}-4{}-a{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}

/// Start the program for `kept`, new or carried on, and hold its input.
fn spawn(app: &Arc<App>, kept: &Kept, fresh: bool) -> Result<Arc<Running>, String> {
    let launcher = app.cfg.launchers.get(&kept.launcher).ok_or_else(|| format!("no launcher called {} is configured", kept.launcher))?;
    let held = launcher.held.as_ref().ok_or_else(|| format!("the launcher {} does not hold its agents", kept.launcher))?;
    let session: Vec<String> = if fresh { &held.fresh } else { &held.resume }.iter().map(|a| a.replace("{id}", &kept.id)).collect();
    let argv: Vec<String> = launcher.argv.iter().flat_map(|a| if a == "{options}" { kept.options.clone() } else { vec![a.replace("{cwd}", &kept.cwd)] }).chain(session).collect();
    let (program, args) = argv.split_first().ok_or("the launcher command is empty")?;
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .current_dir(&kept.cwd)
        // AIRBAND_INTERNAL must not leak in: the agent has to report to the tower. AIRBAND_HELD tells its
        // adapter that this tower holds it, so it says so when it reports in.
        .env_remove("AIRBAND_INTERNAL")
        .env("AIRBAND_HELD", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(false)
        .spawn()
        .map_err(|e| format!("{program} did not start: {e}"))?;
    let input = child.stdin.take().ok_or("its input could not be held")?;
    let output = child.stdout.take();
    let pid = child.id();
    let run = Arc::new(Running { input: tokio::sync::Mutex::new(input), pid });
    running().lock().unwrap().insert(kept.id.clone(), run.clone());
    let id = kept.id.clone();
    tokio::spawn(async move {
        // what it prints is its own business: read and dropped, so it never waits on a full pipe
        if let Some(mut output) = output {
            let mut sink = [0u8; 16 * 1024];
            while matches!(output.read(&mut sink).await, Ok(n) if n > 0) {}
        }
        let status = child.wait().await;
        log(format!("held agent {} ended ({})", &id[..8.min(id.len())], status.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string())));
        // unless it has been started again since: then the one in the list is the new one
        let mut all = running().lock().unwrap();
        if all.get(&id).is_some_and(|r| r.pid == pid) {
            all.remove(&id);
        }
    });
    Ok(run)
}

/// Write one message of his to a held agent as its next turn. One whose process has gone (the tower restarted,
/// or it crashed) is carried on first.
pub(super) async fn tell(app: &Arc<App>, id: &str, text: &str) -> Result<(), String> {
    let run = running().lock().unwrap().get(id).cloned();
    let run = match run {
        Some(run) => run,
        None => {
            let kept = app.inner.lock().unwrap().held.iter().find(|k| k.id == id).cloned().ok_or("the tower no longer holds this agent")?;
            spawn(app, &kept, false)?
        }
    };
    let held = app.inner.lock().unwrap().held.iter().find(|k| k.id == id).and_then(|k| app.cfg.launchers.get(&k.launcher)).and_then(|l| l.held.clone()).ok_or("its launcher is no longer configured")?;
    let line = format!("{}\n", held.line.replace("{text}", &Value::String(text.to_string()).to_string()));
    let mut input = run.input.lock().await;
    input.write_all(line.as_bytes()).await.map_err(|e| format!("its input is closed: {e}"))?;
    input.flush().await.map_err(|e| e.to_string())
}

/// Start a new held agent in `cwd` and give it its task.
pub(super) async fn start(app: &Arc<App>, launcher: &str, cwd: &str, options: Vec<String>, task: &str) -> Result<String, String> {
    let kept = Kept { id: new_id(), cwd: cwd.to_string(), launcher: launcher.to_string(), options };
    spawn(app, &kept, true)?;
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
        if let Err(e) = spawn(app, &k, false) {
            log(format!("could not carry on held agent {}: {e}", &k.id[..8.min(k.id.len())]));
        }
    }
}

/// Let a held agent go: its input is closed, and it ends by itself and signs off.
pub(super) fn end(app: &Arc<App>, id: &str) -> bool {
    let was = running().lock().unwrap().remove(id).is_some();
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
    running().lock().unwrap().remove(id);
}

/// Take over an agent he started in a terminal of his own: end that process and carry the same session on in
/// one the tower holds. Only for an agent whose process the tower knows, so that two never run one session.
pub(super) async fn take_over(app: &Arc<App>, id: &str) -> Result<(), String> {
    let (session, launcher) = {
        let inner = app.inner.lock().unwrap();
        let s = inner.find(id).cloned().ok_or("That agent is no longer running.")?;
        (s, app.cfg.launcher.clone())
    };
    if session.agent.deliver == Some(Deliver::Held) {
        return Err("The tower already runs this one.".into());
    }
    if app.cfg.launchers.get(&launcher).is_none_or(|l| l.held.is_none()) {
        return Err("This computer's launcher does not run agents itself.".into());
    }
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
    spawn(app, &kept, false)?;
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
