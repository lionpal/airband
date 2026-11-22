//! Agents the tower runs itself. An agent he starts in a terminal belongs to that terminal: the tower can only
//! leave it a message from outside, which the agent treats as such. One started here is a process whose input
//! the tower holds, so what he says is written there as his own turn, with nothing around it, and waits its
//! turn in the agent's own queue.
//!
//! Nothing here knows which agent tool it is. The launcher (a tool's row, or one of his own in the settings)
//! says what program to run and which of the two kinds of driver its holder uses (`held.by`): a program that
//! reads a message a line and reports by itself, or one that speaks the Agent Client Protocol. The tower says
//! the same few things to either holder (`hold::Tell`).
//!
//! Each runs in a holder of its own (`airband hold`), a process apart from the tower that keeps the program
//! and its input open and listens on a socket. The tower writes there. So an agent goes on working while the
//! tower is restarted or updated; the tower that starts finds the socket again. Every one is also remembered,
//! so one whose holder has gone (the computer was restarted) is carried on from where its session was.
use super::*;
#[cfg(unix)]
use tokio::io::AsyncReadExt;

/// One held agent, as remembered across restarts.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Kept {
    /// Its session's id, which is also its agent id.
    pub(super) id: String,
    pub(super) cwd: String,
    /// The launcher that started it, by name.
    pub(super) launcher: String,
    /// The model and effort he asked for when he started it, if he did: what it is set to again each time
    /// it is carried on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) effort: Option<String>,
    /// What the agent's own tool calls its session, when that is not `id`: what carries it on, and how the
    /// tower knows a session it sees elsewhere (a tool's own server) is this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) session: Option<String>,
}

/// Is this session, as its own tool calls it, an agent the tower holds? Asked by a watcher that sees every
/// session of its tool, so that one the tower runs is reported by its driver and by nothing else.
pub(super) fn holds(inner: &Inner, session: &str) -> bool {
    inner.held.iter().any(|k| k.session.as_deref() == Some(session))
}

/// The launcher a new agent is started with. For a tool he named, that tool's. With none named: the launcher he
/// set as usual in this computer's settings, and when he set none, the launcher of the coding tool he uses
/// most, as his phone said. Nothing when neither says: he is asked, and nothing is started on a guess.
pub(super) fn launcher_for(app: &App, tool: Option<&str>) -> Option<(String, config::Launcher)> {
    let found = |tool: Option<&str>| app.cfg.launcher_for(tool).map(|(name, l)| (name.clone(), l.clone()));
    match tool {
        Some(tool) => found(Some(tool)),
        None => found(None).or_else(|| {
            let most = app.read().tools.first().cloned();
            most.and_then(|tool| found(Some(&tool)))
        }),
    }
}

/// What a new agent of this launcher can be started on. A program started with arguments takes what its row
/// lists. One that speaks the Agent Client Protocol says for itself each time a session of its opens, and what
/// it last said is what is offered: nothing, until the first of its agents has been started here.
pub(super) fn offers(inner: &Inner, name: &str, launcher: &config::Launcher) -> airband_core::tools::Offers {
    match launcher.held {
        Some(config::Held::Acp { .. }) => inner.menus.get(name).cloned().unwrap_or_default(),
        _ => launcher.offers(),
    }
}

/// An agent that is there: what its tool calls its session, and what it said a session can be set to.
#[cfg_attr(not(unix), allow(dead_code))]
struct Started {
    session: Option<String>,
    offers: airband_core::tools::Offers,
}

/// Remember what a launcher's agent said it can be set to, when it is the agent that says.
fn learn(app: &App, inner: &mut Inner, launcher: &str, offers: airband_core::tools::Offers) {
    if matches!(app.cfg.launchers.get(launcher).and_then(|l| l.held.as_ref()), Some(config::Held::Acp { .. })) {
        inner.menus.insert(launcher.to_string(), offers);
    }
}

/// Where a held agent's holder listens: a short path (a socket's may not be long), of this user's own, that is
/// the same for the same agent of the same tower every time.
fn socket(id: &str) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let name: String = airband_core::encode::hex(&Sha256::digest(format!("{}|{id}", config::dir().display()).as_bytes())[..8]);
    #[cfg(unix)]
    let user = unsafe { libc::getuid() };
    #[cfg(not(unix))]
    let user = 0;
    std::path::PathBuf::from(format!("/tmp/airband-{user}")).join(format!("h-{name}.sock"))
}

/// Say one line to a held agent's holder and read what it answers. Fails when there is none listening: it has
/// ended, or was never started.
#[cfg(unix)]
async fn say(id: &str, line: &[u8]) -> Result<String, String> {
    let mut to = tokio::net::UnixStream::connect(socket(id)).await.map_err(|e| format!("it is not running: {e}"))?;
    to.write_all(line).await.map_err(|e| e.to_string())?;
    to.shutdown().await.map_err(|e| e.to_string())?;
    let mut answer = String::new();
    tokio::time::timeout(Duration::from_secs(5), to.read_to_string(&mut answer)).await.map_err(|_| "its holder did not answer".to_string())?.map_err(|e| e.to_string())?;
    Ok(answer)
}

/// Where a held agent's holder has got to.
#[cfg(unix)]
async fn status(id: &str) -> Result<crate::hold::State, String> {
    let answer = say(id, format!("{}\n", crate::hold::STATUS).as_bytes()).await?;
    serde_json::from_str(answer.trim()).map_err(|e| format!("its holder's answer does not read: {e}"))
}

/// Tell a held agent's holder something.
#[cfg(unix)]
async fn tell_holder(kept: &Kept, tell: crate::hold::Tell) -> Result<(), String> {
    say(&kept.id, format!("{}\n", tell.line()).as_bytes()).await.map(|_| ())
}

/// A session id of the usual shape, made here so the agent is known by it before it has said a word.
pub(super) fn new_id() -> String {
    airband_core::encode::uuid_v4()
}

/// How long a new agent is given to be there. One that speaks the Agent Client Protocol through a program
/// fetched on first use takes a while the first time.
#[cfg(unix)]
const STARTS_WITHIN: Duration = Duration::from_secs(150);

/// Start the program for `kept`, new or carried on, in a holder of its own (`airband hold`): a process apart
/// from the tower, so the agent goes on when the tower is restarted. Returns once the agent is there (the
/// holder says so), with what its tool calls its session; or with why it is not.
#[cfg(unix)]
async fn spawn(app: &Arc<App>, kept: &Kept, fresh: bool) -> Result<Started, String> {
    let launcher = app.cfg.launchers.get(&kept.launcher).ok_or_else(|| format!("no launcher called {} is configured", kept.launcher))?;
    let held = launcher.held.as_ref().ok_or_else(|| format!("the launcher {} does not hold its agents", kept.launcher))?;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let sock = socket(&kept.id);
    if let Some(dir) = sock.parent() {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let mut holder = std::process::Command::new(me);
    holder.arg("hold").arg("--sock").arg(&sock).arg("--cwd").arg(&kept.cwd).arg("--id").arg(&kept.id).arg("--harness").arg(&launcher.harness);
    // whether the program was started under the id the tower chose: one that was reports under it by itself
    let mut given_id = false;
    let program: Vec<String> = match held {
        config::Held::Lines { fresh: new, resume, line, stop } => {
            holder.arg("--by").arg("lines").arg("--line").arg(line);
            if let Some(stop) = stop {
                holder.arg("--stop").arg(stop);
            }
            let mut fill = |a: &String| {
                given_id |= a.contains("{id}");
                a.replace("{cwd}", &kept.cwd).replace("{id}", &kept.id)
            };
            let argv = launcher.command(kept.model.as_deref(), kept.effort.as_deref(), &mut fill);
            let session: Vec<String> = if fresh { new } else { resume }.iter().map(&mut fill).collect();
            argv.into_iter().chain(session).collect()
        }
        config::Held::Acp { program, mode } => {
            holder.arg("--by").arg("acp");
            if let Some(mode) = mode {
                holder.arg("--mode").arg(mode);
            }
            // set by its holder from what the agent itself lists, or it does not start
            if let Some(m) = &kept.model {
                holder.arg("--model").arg(m);
            }
            if let Some(e) = &kept.effort {
                holder.arg("--effort").arg(e);
            }
            if !fresh {
                // carried on by what the agent's own tool calls its session, or not at all
                holder.arg("--session").arg(kept.session.as_ref().ok_or("the tower does not know what its session was called")?);
            }
            program.clone()
        }
    };
    holder
        .arg("--")
        .args(&program)
        // AIRBAND_INTERNAL must not leak in: the agent has to report to the tower. AIRBAND_HELD tells its
        // adapter that this tower holds it, so it says so when it reports in.
        .env_remove("AIRBAND_INTERNAL")
        .env_remove("AIRBAND_HELD_ID")
        .env("AIRBAND_HELD", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // a program that does not know the id reports under whatever its tool calls the session: its adapter is
    // told the id to report under
    if !given_id && matches!(held, config::Held::Lines { .. }) {
        holder.env("AIRBAND_HELD_ID", &kept.id);
    }
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
    let called = program.first().cloned().unwrap_or_default();
    let until = Instant::now() + STARTS_WITHIN;
    let mut listening = false;
    while Instant::now() < until {
        match status(&kept.id).await {
            Ok(crate::hold::State::Ready { session, offers }) => return Ok(Started { session, offers }),
            Ok(crate::hold::State::Failed { why }) => return Err(why),
            Ok(_) => listening = true,
            // its holder has gone again without a word: the program could not be started at all
            Err(_) if listening => return Err(format!("{called} did not start")),
            Err(_) => {}
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    // not there in time: it is not left to turn up later as an agent nobody was told about
    let _ = say(&kept.id, format!("{}\n", crate::hold::Tell::End.line()).as_bytes()).await;
    Err(format!("{called} had not started after {} seconds", STARTS_WITHIN.as_secs()))
}
#[cfg(not(unix))]
async fn spawn(_: &Arc<App>, _: &Kept, _: bool) -> Result<Started, String> {
    Err("airband cannot start agents itself on this system yet".into())
}

/// Carry on a held agent whose holder has gone, and remember what its session is called now.
async fn revive(app: &Arc<App>, kept: &Kept) -> Result<(), String> {
    let started = spawn(app, kept, false).await?;
    let mut inner = app.change();
    if let Some(k) = inner.held.iter_mut().find(|k| k.id == kept.id) {
        k.session = started.session.or(k.session.take());
    }
    learn(app, &mut inner, &kept.launcher, started.offers);
    Ok(())
}

/// Give a held agent one message of his as its next turn. One whose process has gone (the computer restarted,
/// or it crashed) is carried on first.
pub(super) async fn tell(app: &Arc<App>, id: &str, text: &str, lines: &[String]) -> Result<(), String> {
    let receipt = deliver::receipt();
    told_under(app, id, text, &receipt).await?;
    app.passing().receipts.insert(receipt, lines.to_vec());
    Ok(())
}
async fn told_under(app: &Arc<App>, id: &str, text: &str, receipt: &str) -> Result<(), String> {
    let kept = app.read().held.iter().find(|k| k.id == id).cloned().ok_or("airband is no longer running this agent")?;
    #[cfg(unix)]
    {
        let words = || crate::hold::Tell::Say(text.to_string(), receipt.to_string());
        if tell_holder(&kept, words()).await.is_ok() {
            return Ok(());
        }
        // its holder has gone: carry the session on, then say it
        revive(app, &kept).await?;
        tell_holder(&kept, words()).await
    }
    #[cfg(not(unix))]
    {
        let _ = (kept, text, receipt);
        Err("airband cannot start agents itself on this system yet".into())
    }
}

/// A held agent has stopped (it finished its turn, or he stopped it): everything he said to it meanwhile, and
/// did not take back, goes to it now as one turn, in the order he said it.
pub(super) async fn flush(app: &Arc<App>, id: &str) {
    let waiting: Vec<(String, String)> = {
        let mut inner = app.passing();
        let Some(s) = inner.session(id).filter(|s| s.waiting && !s.queue.is_empty()) else { return };
        s.waiting = false;
        let waiting = std::mem::take(&mut s.queue);
        inner.no_longer_waiting(id);
        waiting
    };
    let words = waiting.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>().join("\n\n");
    let told = tell(app, id, &words, &waiting.iter().map(|(line, _)| line.clone()).collect::<Vec<_>>()).await;
    let mut inner = app.change();
    match told {
        Ok(()) => {
            for (line, _) in &waiting {
                if let Some(e) = inner.log.iter_mut().find(|e| e.id == *line && e.held == Some(Held::Queued)) {
                    e.held = None;
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
    inner.tell();
}

/// Stop what a held agent is doing, now. What he said to it meanwhile is then read together as its next turn.
pub(super) async fn stop(app: &Arc<App>, id: &str) -> Result<(), String> {
    let (session, kept) = {
        let inner = app.read();
        let s = inner.find(id).cloned().ok_or("That agent is no longer running.")?;
        if s.agent.deliver != Some(Deliver::Held) {
            return Err("Only an agent airband runs can be stopped from here. Take it over first.".into());
        }
        let kept = inner.held.iter().find(|k| k.id == id).cloned().ok_or("airband is no longer running this agent.")?;
        // a program that reads lines can be stopped only if its launcher says what line stops it
        if matches!(app.cfg.launchers.get(&kept.launcher).and_then(|l| l.held.as_ref()), None | Some(config::Held::Lines { stop: None, .. })) {
            return Err("Its launcher has no way to stop it.".into());
        }
        (s, kept)
    };
    if session.waiting {
        return Err("It is not doing anything.".into());
    }
    #[cfg(unix)]
    tell_holder(&kept, crate::hold::Tell::Stop).await.map_err(|_| "Its process has gone.".to_string())?;
    #[cfg(not(unix))]
    let _ = kept;
    log(format!("stopped {}", session.callsign.text()));
    // a harness may or may not report a stopped turn as finished: either way it is waiting from now
    {
        let mut inner = app.change();
        if let Some(s) = inner.session(id) {
            s.waiting = true;
            s.doing = None;
            s.subagents = 0;
        }
    }
    app.kick.notify_one();
    // let it settle before it is given more
    tokio::time::sleep(Duration::from_millis(400)).await;
    flush(app, id).await;
    Ok(())
}

/// He took back something he said that had not been delivered yet.
pub(super) fn unsend(app: &App, line: &str) -> Result<(), String> {
    let mut inner = app.change();
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
        e.held = Some(Held::Cancelled);
    }
    inner.touch(line);
    inner.tell();
    Ok(())
}

/// Start a new held agent in `cwd` and give it its task. It returns once the agent is there, and not before:
/// one that could not be started is an error that says why, and nothing is left on the radio. With no task it
/// is left waiting for his first word, on the radio at once so he can see it and speak to it.
/// `model` and `effort` are what he asked for it to be set to, as its tool takes them.
pub(super) async fn start(app: &Arc<App>, launcher: &str, cwd: &str, model: Option<String>, effort: Option<String>, task: &str) -> Result<String, String> {
    let mut kept = Kept { id: new_id(), cwd: cwd.to_string(), launcher: launcher.to_string(), model, effort, session: None };
    let started = spawn(app, &kept, true).await?;
    kept.session = started.session;
    {
        let mut inner = app.change();
        learn(app, &mut inner, launcher, started.offers);
        inner.held.push(kept.clone());
        if task.is_empty() {
            let harness = app.cfg.launchers.get(launcher).map(|l| l.harness.clone()).unwrap_or_default();
            inner.check_in(&Agent { id: kept.id.clone(), harness, cwd: Some(cwd.to_string()), name: None, title: None, pid: None, deliver: Some(Deliver::Held), meta: Default::default() });
            if let Some(s) = inner.session(&kept.id) {
                s.waiting = true;
            }
        }
    }
    if task.is_empty() {
        app.kick.notify_one();
    } else {
        tell(app, &kept.id, task, &[]).await?;
    }
    Ok(kept.id)
}

/// The tower has started: carry on every agent it was holding, so each is there to be spoken to. One whose
/// session cannot be carried on is let go.
pub(super) async fn carry_on(app: &Arc<App>) {
    let kept: Vec<Kept> = app.read().held.clone();
    for k in kept {
        // only the ones still on the radio: one that signed off is finished
        if app.read().find(&k.id).is_none() {
            continue;
        }
        // its holder outlived the last tower: it is still running, and there is nothing to do
        #[cfg(unix)]
        if status(&k.id).await.is_ok() {
            continue;
        }
        if let Err(e) = revive(app, &k).await {
            log(format!("could not carry on held agent {}: {e}", &k.id[..8.min(k.id.len())]));
        }
    }
}

/// Let a held agent go: its input is closed, and it ends by itself and signs off.
pub(super) async fn end(app: &Arc<App>, id: &str) -> bool {
    let kept = app.read().held.iter().find(|k| k.id == id).cloned();
    #[cfg(unix)]
    let was = match &kept {
        Some(kept) => tell_holder(kept, crate::hold::Tell::End).await.is_ok(),
        None => false,
    };
    #[cfg(not(unix))]
    let was = false;
    let _ = &kept;
    let mut inner = app.change();
    let before = inner.held.len();
    inner.held.retain(|k| k.id != id);
    was || inner.held.len() != before
}

/// An agent has signed off: if the tower held it, it holds it no more.
pub(super) fn forget(inner: &mut Inner, id: &str) {
    inner.held.retain(|k| k.id != id);
}

/// Take over an agent he started in a terminal of his own: end that process and carry the same session on in
/// one the tower holds. Only for an agent whose process the tower knows, so that two never run one session.
pub(super) async fn take_over(app: &Arc<App>, id: &str) -> Result<(), String> {
    let (session, launcher) = {
        let inner = app.read();
        let s = inner.find(id).cloned().ok_or("That agent is no longer running.")?;
        // carried on by a launcher that runs this agent's own harness: never one tool's session under another
        let launcher = app.cfg.launcher_for(Some(&s.agent.harness)).filter(|(_, l)| l.held.is_some()).map(|(name, _)| name.clone());
        (s, launcher)
    };
    if session.agent.deliver == Some(Deliver::Held) {
        return Err("airband already runs this one.".into());
    }
    let Some(launcher) = launcher else {
        return Err(format!("No launcher here runs {} agents itself.", session.agent.harness));
    };
    let cwd = session.agent.cwd.clone().ok_or("airband does not know which folder it is in.")?;
    let pid = session.agent.pid.ok_or("airband cannot find it running on this computer. Close it in its terminal, then start it from here.")?;
    if !session.waiting {
        return Err("It is in the middle of something. Take it over when it has stopped.".into());
    }
    // ask it to end, and wait for it to be gone: two processes must never run one session
    terminate(pid);
    for _ in 0..50 {
        if !alive(pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    if alive(pid) {
        return Err("Its terminal session did not end. Close it there, then try again.".into());
    }
    let mut kept = Kept { id: id.to_string(), cwd, launcher, model: None, effort: None, session: Some(id.to_string()) };
    // ending it signed it off; it reports in again from the new process, under the same id
    let started = spawn(app, &kept, false).await?;
    kept.session = started.session.or(kept.session);
    let mut inner = app.change();
    learn(app, &mut inner, &kept.launcher, started.offers);
    inner.held.retain(|k| k.id != id);
    inner.held.push(kept);
    // it is on the radio from now, before it has reported in: held, and reachable
    let mut agent = session.agent.clone();
    agent.deliver = Some(Deliver::Held);
    agent.pid = None;
    inner.check_in(&agent);
    if !inner.keep_name(id, session.callsign.clone()) {
        log(format!("{} was taken while it moved, so it keeps the name it came back with", session.callsign.text()));
    }
    if let Some(s) = inner.session(id) {
        s.agent.deliver = Some(Deliver::Held);
        s.agent.pid = None;
        s.waiting = true;
    }
    inner.tell();
    log(format!("took over {}", session.callsign.text()));
    Ok(())
}
