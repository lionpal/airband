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
pub(super) use crate::hold::NotStarted;

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

/// What a new agent of this launcher can be started on. A program started with arguments takes what its row
/// lists. One that speaks the Agent Client Protocol says for itself each time a session of its opens, and what
/// it last said is what is offered; until the first of its agents has said, nothing is (what he names goes to
/// the agent as he said it, and one it does not have is refused when it starts).
pub(super) fn offers(inner: &Inner, name: &str, launcher: &config::Launcher) -> airband_core::tools::Offers {
    if launcher.says_its_settings() { inner.menus.get(name).cloned().unwrap_or_default() } else { launcher.offers() }
}

/// An agent that is there: what its tool calls its session, and what it said a session can be set to.
struct Started {
    session: Option<String>,
    offers: airband_core::tools::Offers,
}

/// Remember what a launcher's agent said it can be set to, when it is the agent that says.
fn learn(app: &App, inner: &mut Inner, launcher: &str, offers: airband_core::tools::Offers) {
    if app.cfg().launchers.get(launcher).is_some_and(config::Launcher::says_its_settings) {
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
/// Holders run on unix only: elsewhere there is never one listening.
#[cfg(not(unix))]
async fn say(_: &str, _: &[u8]) -> Result<String, String> {
    Err(NOT_HERE.into())
}
/// Why an agent cannot be held on this system.
const NOT_HERE: &str = "airband cannot start agents itself on this system yet";

/// Where a held agent's holder has got to.
async fn status(id: &str) -> Result<crate::hold::State, String> {
    let answer = say(id, format!("{}\n", crate::hold::status_line()).as_bytes()).await?;
    serde_json::from_str(answer.trim()).map_err(|e| format!("its holder's answer does not read: {e}"))
}

/// Tell a held agent's holder something.
async fn tell_holder(kept: &Kept, tell: crate::hold::Tell) -> Result<(), String> {
    let answer = say(&kept.id, format!("{}\n", tell.line()).as_bytes()).await?;
    taken(&answer)
}
/// Whether a holder's answer says it took what it was told: only a typed `taken` counts.
fn taken(answer: &str) -> Result<(), String> {
    match serde_json::from_str::<crate::hold::Answer>(answer.trim()) {
        Ok(crate::hold::Answer::Taken) => Ok(()),
        Ok(crate::hold::Answer::Refused { why }) => Err(why),
        Err(e) => Err(format!("its holder's answer does not read: {e}")),
    }
}

/// A session id of the usual shape, made here so the agent is known by it before it has said a word.
pub(super) fn new_id() -> String {
    airband_core::encode::uuid_v4()
}

/// How long a new agent is given to be there. One that speaks the Agent Client Protocol through a program
/// fetched on first use takes a while the first time.
const STARTS_WITHIN: Duration = Duration::from_secs(150);

/// Start the program for `kept`, new or carried on, in a holder of its own (`airband hold`): a process apart
/// from the tower, so the agent goes on when the tower is restarted. Returns once the agent is there (the
/// holder says so), with what its tool calls its session; or with why it is not. Holders run on unix only.
async fn spawn(app: &Arc<App>, kept: &Kept, fresh: bool) -> Result<Started, NotStarted> {
    if !cfg!(unix) {
        return Err(NotStarted::Told(NOT_HERE.into()));
    }
    spawn_with(app, &*app.procs, kept, fresh).await
}

/// What starts a held agent's holder: this program's `hold`, told the socket, the folder, the program and how
/// its turns are given. Nothing is started here.
fn holder_run(cfg: &config::Config, me: std::path::PathBuf, sock: &std::path::Path, kept: &Kept, fresh: bool) -> Result<(edges::procs::Run, Vec<String>), String> {
    let launcher = cfg.launchers.get(&kept.launcher).ok_or_else(|| format!("no launcher called {} is configured", kept.launcher))?;
    let held = launcher.held.as_ref().ok_or_else(|| format!("the launcher {} does not hold its agents", kept.launcher))?;
    let mut holder = edges::procs::Run { program: me, ..Default::default() };
    holder.arg("hold").arg("--sock").arg(sock).arg("--cwd").arg(&kept.cwd).arg("--id").arg(&kept.id).arg("--harness").arg(&launcher.harness);
    // whether the program was started under the id the tower chose: one that was reports under it by itself
    let mut given_id = false;
    holder.arg("--by").arg(crate::hold::By::of(held).word());
    let program: Vec<String> = match held {
        config::Held::Lines { fresh: new, resume, line, stop } => {
            holder.arg("--line").arg(line);
            if let Some(stop) = stop {
                holder.arg("--stop").arg(stop);
            }
            let mut fill = |a: &String| {
                given_id |= a.contains("{id}");
                airband_core::template::filled(a, &[("cwd", &kept.cwd), ("id", &kept.id)])
            };
            let argv = launcher.command(kept.model.as_deref(), kept.effort.as_deref(), &mut fill);
            let session: Vec<String> = if fresh { new } else { resume }.iter().map(&mut fill).collect();
            argv.into_iter().chain(session).collect()
        }
        config::Held::Acp { program, mode } => {
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
    holder.arg("--");
    for a in &program {
        holder.arg(a);
    }
    // AIRBAND_INTERNAL must not leak in: the agent has to report to the tower. AIRBAND_HELD tells its adapter
    // that this tower holds it, so it says so when it reports in.
    holder.unset = vec![INTERNAL_ENV.into(), "AIRBAND_HELD_ID".into()];
    holder.set.push(("AIRBAND_HELD".into(), "1".into()));
    // a program that does not know the id reports under whatever its tool calls the session: its adapter is
    // told the id to report under
    if !given_id && matches!(held, config::Held::Lines { .. }) {
        holder.set.push(("AIRBAND_HELD_ID".into(), kept.id.clone()));
    }
    Ok((holder, program))
}

async fn spawn_with(app: &Arc<App>, procs: &dyn edges::procs::Procs, kept: &Kept, fresh: bool) -> Result<Started, NotStarted> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let sock = socket(&kept.id);
    let (holder, program) = holder_run(&app.cfg(), me, &sock, kept, fresh)?;
    if let Some(dir) = sock.parent() {
        airband_io::files::private_folder(dir).map_err(|e| format!("cannot make {} private: {e}", dir.display()))?;
    }
    airband_io::files::remove(&sock).ok();
    procs.start_apart(holder).map_err(|e| format!("its holder did not start: {e}"))?;
    let called = program.first().cloned().unwrap_or_default();
    let until = Instant::now() + STARTS_WITHIN;
    let mut listening = false;
    while Instant::now() < until {
        match status(&kept.id).await {
            Ok(crate::hold::State::Ready { session, offers }) => return Ok(Started { session, offers }),
            Ok(crate::hold::State::Failed { why }) => return Err(NotStarted::Failed(why)),
            Ok(crate::hold::State::NoSuch(no)) => return Err(NotStarted::NoSuch(no)),
            Ok(_) => listening = true,
            // its holder has gone again without a word: the program could not be started at all
            Err(_) if listening => return Err(format!("{called} did not start").into()),
            Err(_) => {}
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    // not there in time: it is not left to turn up later as an agent nobody was told about
    // a holder that turns up late and cannot be told to end is said: it may run on with nobody told of it
    if let Err(e) = say(&kept.id, format!("{}\n", crate::hold::Tell::End.line()).as_bytes()).await {
        log(format!("{called} was late and could not be told to end ({e}); it may still start"));
    }
    Err(format!("{called} had not started after {} seconds", STARTS_WITHIN.as_secs()).into())
}

/// Carry on a held agent whose holder has gone, and remember what its session is called now.
async fn revive(app: &Arc<App>, kept: &Kept) -> Result<(), String> {
    let started = spawn(app, kept, false).await.map_err(|e| e.to_string())?;
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
    app.change().receipts.insert(receipt, lines.to_vec());
    Ok(())
}
async fn told_under(app: &Arc<App>, id: &str, text: &str, receipt: &str) -> Result<(), String> {
    let kept = app.read().kept_held(id).ok_or("airband is no longer running this agent")?;
    let words = || crate::hold::Tell::Say(text.to_string(), receipt.to_string());
    if tell_holder(&kept, words()).await.is_ok() {
        return Ok(());
    }
    // its holder has gone: carry the session on, then say it. One at a time: another delivery may have
    // started it again while this one waited, and then it is only told
    let _one = app.reviving.lock().await;
    if tell_holder(&kept, words()).await.is_ok() {
        return Ok(());
    }
    revive(app, &kept).await?;
    tell_holder(&kept, words()).await
}

/// A held agent has stopped (it finished its turn, or he stopped it): everything he said to it meanwhile, and
/// did not take back, goes to it now as one turn, in the order he said it.
pub(super) async fn flush(app: &Arc<App>, id: &str) {
    let waiting: Vec<(String, String)> = {
        let mut inner = app.change();
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
                // a line said to several waits until the last of them has it
                if inner.sessions.iter().any(|s| s.queue.iter().any(|(l, _)| l == line)) {
                    continue;
                }
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
        let s = inner.find(id).cloned().ok_or(NOT_RUNNING)?;
        if s.agent.deliver != Some(Deliver::Held) {
            return Err("Only an agent airband runs can be stopped from here. Take it over first.".into());
        }
        let kept = inner.kept_held(id).ok_or("airband is no longer running this agent.")?;
        // a program that reads lines can be stopped only if its launcher says what line stops it
        if matches!(app.cfg().launchers.get(&kept.launcher).and_then(|l| l.held.as_ref()), None | Some(config::Held::Lines { stop: None, .. })) {
            return Err("Its launcher has no way to stop it.".into());
        }
        (s, kept)
    };
    if session.waiting {
        return Err("It is not doing anything.".into());
    }
    tell_holder(&kept, crate::hold::Tell::Stop).await.map_err(|_| "Its process has gone.".to_string())?;
    log(format!("stopped {}", short_id(&session.agent.id)));
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
    // its holder has written the stop to it (it said `taken`), and writes what comes next behind it: the program
    // reads the stop first, so what he said meanwhile can go at once
    flush(app, id).await;
    Ok(())
}

/// He took back something he said that had not been delivered yet.
pub(super) fn unsend(app: &App, line: &str) -> Result<(), String> {
    let mut inner = app.change();
    let mut held_by: Vec<String> = Vec::new();
    for s in inner.sessions.iter_mut() {
        let before = s.queue.len();
        s.queue.retain(|(id, _)| id != line);
        if s.queue.len() != before {
            held_by.push(s.agent.id.clone());
        }
    }
    if held_by.is_empty() {
        return Err("It has already been delivered.".into());
    }
    // taken back from the ones still waiting for it; one said to others too stands as said to them
    if let Some(e) = inner.log.iter_mut().find(|e| e.id == line) {
        let reached_others = e.to_agents.iter().any(|a| !held_by.contains(a));
        e.held = if reached_others { None } else { Some(Held::Cancelled) };
    }
    inner.touch(line);
    inner.tell();
    Ok(())
}

impl Inner {
    /// What is kept about an agent the tower runs, by its id.
    pub(super) fn kept_held(&self, id: &str) -> Option<Kept> {
        self.held.iter().find(|k| k.id == id).cloned()
    }
}

/// Start a new held agent in `cwd` and give it its task. It returns once the agent is there, and not before:
/// one that could not be started is an error that says why, and nothing is left on the radio. With no task it
/// is left waiting for his first word, on the radio at once so he can see it and speak to it.
/// `model` and `effort` are what he asked for it to be set to, as its tool takes them.
/// `line` is the chat line the task is written as, so the agent taking it in marks that line seen.
/// Why it did not start is [`NotStarted`]: what he is told, and what only the log is.
pub(super) async fn start(app: &Arc<App>, launcher: &str, cwd: &str, model: Option<String>, effort: Option<String>, task: &str, line: &str) -> Result<String, NotStarted> {
    let harness = app.cfg().launchers.get(launcher).map(|l| l.harness.clone()).ok_or_else(|| format!("no launcher called {launcher} is configured"))?;
    // nothing is started that no team could hold: it would run with nobody able to speak to it
    if !a_team_can_hold(&app.read(), cwd) {
        return Err(NotStarted::Told("That folder has no name a team could have.".into()));
    }
    let mut kept = Kept { id: new_id(), cwd: cwd.to_string(), launcher: launcher.to_string(), model, effort, session: None };
    let started = spawn(app, &kept, true).await?;
    kept.session = started.session;
    let taken = {
        let mut inner = app.change();
        learn(app, &mut inner, launcher, started.offers);
        inner.held.push(kept.clone());
        let taken = task.is_empty().then(|| inner.check_in(&Agent { id: kept.id.clone(), harness, cwd: Some(cwd.to_string()), name: None, title: None, pid: None, deliver: Some(Deliver::Held), meta: Default::default() }));
        if let Some(s) = inner.session(&kept.id) {
            s.waiting = true;
        }
        taken
    };
    if taken == Some(None) {
        end(app, &kept.id).await;
        return Err(NotStarted::Told("It has no team airband can put it in.".into()));
    }
    if task.is_empty() {
        app.kick.notify_one();
    } else if let Err(why) = tell(app, &kept.id, task, &[line.to_string()]).await {
        // started for this task and it could not be given: it is not left running without it
        end(app, &kept.id).await;
        return Err(NotStarted::Failed(why));
    }
    Ok(kept.id)
}

/// The tower has started: carry on every agent it was holding, so each is there to be spoken to. One whose
/// session cannot be carried on is let go.
pub(super) async fn carry_on(app: &Arc<App>) {
    // only the ones still on the radio: one that signed off is finished, and nothing of it is kept
    let kept: Vec<Kept> = {
        let mut inner = app.change();
        inner.drop_held_off_the_radio();
        inner.held.clone()
    };
    let disk = app.read().disk.clone();
    for k in to_carry_on(kept, &*disk) {
        // one at a time with a delivery that may be starting it too
        let _one = app.reviving.lock().await;
        // its holder outlived the last tower, or a delivery started it: it is running, and there is nothing to do
        if status(&k.id).await.is_ok() {
            continue;
        }
        if let Err(e) = revive(app, &k).await {
            log(format!("could not carry on held agent {}: {e}", short_id(&k.id)));
        }
    }
}

/// The held agents worth carrying on: the ones whose folder is still there. One whose folder has gone could
/// not be started in it, and is said and left.
fn to_carry_on(kept: Vec<Kept>, folders: &dyn edges::folders::Folders) -> Vec<Kept> {
    kept.into_iter()
        .filter(|k| {
            let there = folders.exists(&k.cwd);
            if !there {
                log(format!("did not carry on held agent {}: its folder is gone", short_id(&k.id)));
            }
            there
        })
        .collect()
}

impl Inner {
    /// Let go of what is kept of held agents that are no longer on the radio.
    pub(super) fn drop_held_off_the_radio(&mut self) {
        let on: HashSet<String> = self.sessions.iter().map(|s| s.agent.id.clone()).collect();
        self.held.retain(|k| on.contains(&k.id));
    }
}

/// Whether a team could hold an agent working in this folder: the one there, or one the folder could name.
pub(super) fn a_team_can_hold(inner: &Inner, cwd: &str) -> bool {
    inner.team_at(cwd).is_some() || inner.name_for_folder(cwd).is_some()
}

/// Let a held agent go: its input is closed, and it ends by itself and signs off.
pub(super) async fn end(app: &Arc<App>, id: &str) -> bool {
    let kept = app.read().kept_held(id);
    let was = match &kept {
        Some(kept) => tell_holder(kept, crate::hold::Tell::End).await.is_ok(),
        None => false,
    };
    let mut inner = app.change();
    let before = inner.held.len();
    inner.held.retain(|k| k.id != id);
    was || inner.held.len() != before
}

/// An agent has signed off: if the tower held it, it holds it no more.
pub(super) fn forget(inner: &mut Inner, id: &str) {
    inner.held.retain(|k| k.id != id);
}

/// The launcher that carries on an agent of `harness` the tower holds: of the launchers of that harness that
/// hold their agents, by the one rule (`launch::which_launcher`): his usual launcher when it is one of them,
/// else the only one, else he is asked.
fn holder_for(cfg: &config::Config, harness: &str) -> launch::Starts {
    let holding: Vec<LauncherId> = cfg.launchers.iter().filter(|(_, l)| l.harness == harness && l.held.is_some()).map(|(name, _)| LauncherId::new(name.as_str())).collect();
    launch::which_launcher(cfg.launcher.as_ref(), &holding)
}

/// The coding tools whose agents a launcher here would carry on, by the one rule (`holder_for`): what a
/// take-over needs of this computer's settings.
pub(super) fn carried_on(cfg: &config::Config) -> Vec<String> {
    let mut tools: Vec<String> = cfg.launchers.values().filter(|l| l.held.is_some()).map(|l| l.harness.clone()).filter(|h| matches!(holder_for(cfg, h), launch::Starts::This(_))).collect();
    tools.sort();
    tools.dedup();
    tools
}

/// Whether the tower can take this agent over as things are: it does not run it already, a launcher here
/// carries on its tool's agents (`carried`, from [`carried_on`]), and it knows the agent's folder, its process
/// and a team to put it in. Whether it is in the middle of something is asked at the moment he taps.
pub(super) fn can_take_over(inner: &Inner, s: &Session, carried: &[String]) -> bool {
    s.agent.deliver != Some(Deliver::Held) && carried.contains(&s.agent.harness) && s.agent.pid.is_some() && s.agent.cwd.as_deref().is_some_and(|cwd| a_team_can_hold(inner, cwd))
}

/// Take over an agent he started in a terminal of his own: end that process and carry the same session on in
/// one the tower holds. Only for an agent whose process the tower knows, so that two never run one session.
pub(super) async fn take_over(app: &Arc<App>, id: &str) -> Result<(), String> {
    let (session, launcher) = {
        let inner = app.read();
        let s = inner.find(id).cloned().ok_or(NOT_RUNNING)?;
        // carried on by a launcher that runs this agent's own harness: never one tool's session under another
        let launcher = holder_for(&app.cfg(), &s.agent.harness);
        (s, launcher)
    };
    if session.agent.deliver == Some(Deliver::Held) {
        return Err("airband already runs this one.".into());
    }
    let launcher = match launcher {
        launch::Starts::This(launcher) => launcher.to_string(),
        launch::Starts::Nobody => return Err(format!("No launcher here runs {} agents itself.", session.agent.harness)),
        // the take-over is a button, which carries no answer back: he is told the names to choose between
        launch::Starts::Which(several) => {
            let named: Vec<String> = launch::sayable(&app.cfg(), &several).into_iter().map(|(_, n)| n).collect();
            return Err(format!("It could be carried on by {}, and none of them is the usual one, so it was left running in its terminal.", super::route::either(&named)));
        }
    };
    let cwd = session.agent.cwd.clone().ok_or("airband does not know which folder it is in.")?;
    let pid = session.agent.pid.ok_or("airband cannot find it running on this computer. Close it in its terminal, then start it from here.")?;
    if !session.waiting {
        return Err("It is in the middle of something. Take it over when it has stopped.".into());
    }
    // nothing is ended that the tower could not then hold: a team must be there for it before its terminal
    // session goes
    if !a_team_can_hold(&app.read(), &cwd) {
        return Err("It has no team airband can put it in, so it was left running in its terminal.".into());
    }
    // ending it signs it off: written down first is the team it goes back to, by whoever put it there
    app.change().put(id, &session.team, session.moved.unwrap_or(Mover::Tower));
    // ask it to end, and wait for it to be gone: two processes must never run one session, so the one the
    // tower holds can only start once this one has ended
    if edges::procs::end_and_wait(&*app.procs, pid).await.is_err() {
        return Err("Its terminal session did not end. Close it there, then try again.".into());
    }
    let mut kept = Kept { id: id.to_string(), cwd, launcher, model: None, effort: None, session: Some(id.to_string()) };
    // ending it signed it off; it reports in again from the new process, under the same id
    let started = spawn(app, &kept, false).await.map_err(|e| format!("Its terminal session ended, but airband could not carry it on: {e}"))?;
    kept.session = started.session.or(kept.session);
    let taken = {
        let mut inner = app.change();
        learn(app, &mut inner, &kept.launcher, started.offers);
        inner.held.retain(|k| k.id != id);
        inner.held.push(kept);
        // it is on the radio from now, before it has reported in: held, and reachable
        let mut agent = session.agent.clone();
        agent.deliver = Some(Deliver::Held);
        agent.pid = None;
        let taken = inner.check_in(&agent).is_some();
        if taken {
            if !inner.keep_name(id, session.callsign.clone()) {
                log(format!("{} was taken while it moved, so it keeps the name it came back with", short_id(id)));
            }
            if let Some(s) = inner.session(id) {
                s.agent.deliver = Some(Deliver::Held);
                s.agent.pid = None;
                s.waiting = true;
            }
            inner.tell();
        }
        taken
    };
    if !taken {
        // the session goes on in the holder just started: let it go, so nothing runs that nobody can speak to
        end(app, id).await;
        return Err("It has no team airband can put it in, so it was stopped.".into());
    }
    log(format!("took over {}", short_id(id)));
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn what_is_kept_of_a_held_agent_off_the_radio_is_let_go() {
        let mut inner = Inner::default();
        let kept = |id: &str| Kept { id: id.into(), cwd: "/w".into(), launcher: "l".into(), model: None, effort: None, session: None };
        inner.held = vec![kept("on"), kept("off")];
        let on = airband_core::agent::Agent { cwd: Some("/w/shop".into()), ..crate::adapters::agent_with("on", "test") };
        inner.check_in(&on).expect("taken in");
        inner.drop_held_off_the_radio();
        assert_eq!(inner.held.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(), ["on"]);
    }
    use super::*;

    #[test]
    fn a_take_over_is_carried_on_by_a_launcher_that_holds_its_agents_though_the_usual_one_does_not() {
        let held = serde_json::json!({ "by": "lines", "fresh": [], "resume": [], "line": "{text}", "stop": "{id}" });
        let cfg = |usual: &str, launchers: serde_json::Value| crate::tower::http::tests::app_with("holder", serde_json::json!({ "launcher": usual, "launchers": launchers })).cfg();
        // his usual hands its agents off: the one that holds them carries it on
        let one = cfg("mine", serde_json::json!({ "mine": { "harness": "tool-one", "argv": ["true"] }, "held-one": { "harness": "tool-one", "argv": ["true"], "held": held } }));
        assert_eq!(holder_for(&one, "tool-one"), launch::Starts::This(LauncherId::new("held-one")));
        // his usual holds them: that one, whichever others do
        let usual = cfg("b", serde_json::json!({ "a": { "harness": "tool-one", "argv": ["true"], "held": held }, "b": { "harness": "tool-one", "argv": ["true"], "held": held } }));
        assert_eq!(holder_for(&usual, "tool-one"), launch::Starts::This(LauncherId::new("b")));
        // several hold them and none is usual: he is asked
        let several = cfg("other", serde_json::json!({ "other": { "harness": "tool-two", "argv": ["true"] }, "a": { "harness": "tool-one", "argv": ["true"], "held": held }, "b": { "harness": "tool-one", "argv": ["true"], "held": held } }));
        assert_eq!(holder_for(&several, "tool-one"), launch::Starts::Which(vec![LauncherId::new("a"), LauncherId::new("b")]));
        // none of another tool's launchers, however they are held
        assert_eq!(holder_for(&several, "tool-three"), launch::Starts::Nobody);
    }

    #[test]
    fn a_line_taken_back_from_one_agent_still_stands_for_another_it_reached() {
        let app = crate::tower::http::tests::app("unsend-some");
        {
            let mut inner = app.change();
            for id in ["a", "b"] {
                inner.check_in(&airband_core::agent::Agent { cwd: Some("/w/shop".into()), ..crate::adapters::agent_with(id, "test") }).unwrap();
            }
            inner.session("a").unwrap().queue = vec![("l1".into(), "words".into()), ("l2".into(), "more".into())];
            inner.log.push(Entry { id: "l1".into(), his: true, to_agents: vec!["a".into(), "b".into()], held: Some(Held::Queued), ..Entry::default() });
            inner.log.push(Entry { id: "l2".into(), his: true, to_agents: vec!["a".into()], held: Some(Held::Queued), ..Entry::default() });
        }
        unsend(&app, "l1").unwrap();
        unsend(&app, "l2").unwrap();
        let inner = app.read();
        let held = |id: &str| inner.log.iter().find(|e| e.id == id).unwrap().held;
        assert_eq!(held("l1"), None, "b has it: it stands as said to b");
        assert_eq!(held("l2"), Some(Held::Cancelled));
        assert!(inner.find("a").unwrap().queue.is_empty());
    }

    #[test]
    fn words_count_as_given_only_when_the_holder_says_it_took_them() {
        assert!(taken(r#"{"answer":"taken"}"#).is_ok());
        assert_eq!(taken(r#"{"answer":"refused","why":"it has ended"}"#), Err("it has ended".to_string()));
        assert!(taken("").is_err(), "a holder that answers nothing took nothing");
    }
    use edges::folders::FakeFolders;

    fn kept(id: &str, cwd: &str) -> Kept {
        Kept { id: id.into(), cwd: cwd.into(), launcher: "l".into(), model: None, effort: None, session: None }
    }

    #[test]
    fn only_held_agents_whose_folder_is_still_there_are_carried_on() {
        let disk = FakeFolders(vec!["/work/shop".into()]);
        let carried = to_carry_on(vec![kept("a", "/work/shop"), kept("b", "/work/gone")], &disk);
        assert_eq!(carried.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn a_holder_is_told_its_socket_folder_and_program_and_the_agent_its_id_when_the_program_does_not_know_it() {
        let held = airband_core::tools::Held::Lines { fresh: vec!["--new".into()], resume: vec!["--again".into()], line: "{text}".into(), stop: None };
        let launcher = airband_core::tools::Launcher { name: "l".into(), harness: "tool".into(), argv: vec!["tool".into(), "--in".into(), "{cwd}".into()], held: Some(held), ..Default::default() };
        let mut cfg: config::Config = serde_json::from_str(r#"{"token":"own","radio_token":"radio"}"#).unwrap();
        cfg.launchers = [("l".to_string(), launcher)].into();
        let (run, program) = holder_run(&cfg, "/bin/airband".into(), std::path::Path::new("/s/a.sock"), &kept("a", "/work/shop"), true).unwrap();
        assert_eq!(program, ["tool", "--in", "/work/shop", "--new"]);
        let words = run.words();
        assert_eq!(&words[..9], ["hold", "--sock", "/s/a.sock", "--cwd", "/work/shop", "--id", "a", "--harness", "tool"]);
        assert_eq!(words[words.len() - 5..], ["--", "tool", "--in", "/work/shop", "--new"]);
        assert!(run.set.contains(&("AIRBAND_HELD_ID".into(), "a".into())), "the program was not given the id");
        assert!(run.unset.contains(&INTERNAL_ENV.into()));
        let procs = edges::procs::FakeProcs::ending_after(None);
        edges::procs::Procs::start_apart(&procs, run.clone()).unwrap();
        assert_eq!(procs.started.lock().unwrap().as_slice(), [run]);
        // carried on, a program with no launcher of that name is never started
        let unknown = Kept { launcher: "gone".into(), ..kept("b", "/work/shop") };
        assert!(holder_run(&cfg, "/bin/airband".into(), std::path::Path::new("/s/b.sock"), &unknown, false).is_err());
    }

    #[test]
    fn a_folder_no_team_could_hold_is_never_one_an_agent_is_started_or_taken_over_in() {
        let inner = Inner::default();
        assert!(!a_team_can_hold(&inner, "/"));
    }
}
