//! Running the tower: its keys settled, its state opened, everything it watches started, and its addresses served.
use super::*;

pub(crate) fn run(cfg: Config) -> Result<(), String> {
    if let Some(why) = config::keyless() {
        return Err(format!("airband's keys could not be read: {why}"));
    }
    settle_keys();
    stop_whole_on_failure();
    keep_catalogue_fresh();
    start_local_model(&cfg);
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async move {
        let port = cfg.port;
        let app = open(cfg)?;
        let router = http::router(app.clone());
        tokio::spawn(pump_loop(app.clone()));
        tokio::spawn(accounts::keep_alive(app.clone()));
        tokio::spawn(progress_loop(app.clone()));
        tokio::spawn(activity_loop(app.clone()));
        held::carry_on(&app).await;
        watch_tools(&app);
        read_worked_folders(&app);
        drain_spool(&app);
        relay_link::start(app.clone(), router.clone(), tokio::runtime::Handle::current());
        tokio::spawn(tidy(app.clone()));
        serve(app, router, port).await
    })
}

/// Keys a run without the keychain had to seal in a file go into the keychain when this one can use it.
fn settle_keys() {
    if crate::vault::carry_asked().exists() {
        // asked for from a shell that could not read the keychain: this run can
        let _ = std::fs::remove_file(crate::vault::carry_asked());
        match crate::vault::carry() {
            Ok(true) => log("the keys are out of the keychain and sealed in a file, ready for a copy signed with another certificate"),
            Ok(false) => {}
            Err(e) => log(format!("the keys could not be carried out of the keychain: {e}")),
        }
    } else if crate::vault::settle() {
        log("the keys are now in the keychain");
    }
}

/// A tower with one part dead answers nothing and looks alive. Anything that goes wrong badly enough to stop a
/// thread stops the tower, which is then started again (launchd, systemd), whole.
fn stop_whole_on_failure() {
    let usual = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        usual(info);
        log(format!("the tower stopped itself after an internal failure: {info}"));
        std::process::exit(101);
    }));
}

/// A newer catalogue, now and twice a day: what a phone is offered for this computer comes from it.
fn keep_catalogue_fresh() {
    std::thread::spawn(|| loop {
        let (changed, lines) = crate::catalog::refresh();
        lines.into_iter().for_each(log);
        if changed {
            log(format!("catalogue: now rev {}", airband_core::catalog::current().rev));
        }
        std::thread::sleep(Duration::from_secs(12 * 3600));
    });
}

/// A model on this computer is started now, so it is ready when the first message comes; until it is, a message
/// is given as written.
fn start_local_model(cfg: &Config) {
    local::listen_after(cfg.port);
    if let Some((model, server)) = judge::local_model(&cfg.summarizer) {
        std::thread::spawn(move || {
            match local::ensure(&model, &server, true) {
                Ok(_) => log("local model: ready"),
                Err(why) => log(format!("local model: {why}")),
            }
        });
    }
}

/// The tower's state as it was kept, settled for this run, and written again if settling changed it.
fn open(cfg: Config) -> Result<Arc<App>, String> {
    let store = Store::open(&config::dir().join("airband.db"))?;
    let (saved, entries) = store.load(LOG_KEEP)?;
    let (seq, rev) = store.last_counts()?;
    let mut inner = Inner { sessions: saved.sessions, teams: saved.teams, here: config::station(&cfg), held: saved.held, menus: saved.menus, tools: saved.tools, placed: saved.placed, seq, rev, log: entries, ..Inner::default() };
    inner.teams_at_start(saved.projects, saved.named_teams, &cfg.projects);
    inner.links = links::Book::load();
    inner.reap();
    inner.close_requests_from_before();
    // credentials said before the chat masked them: masked now, and sent to every radio again as they are
    let leaked: Vec<String> = inner.log.iter_mut().filter_map(|e| e.scrub().then(|| e.id.clone())).collect();
    if !leaked.is_empty() {
        log(format!("masked a credential in {} kept message{}", leaked.len(), if leaked.len() == 1 { "" } else { "s" }));
        leaked.iter().for_each(|id| inner.touch(id));
    }
    pictures::prune();
    attachments::prune();
    inner.settle_reading();
    let push = push::Wake::load(&store)?;
    let app = Arc::new(App { push, cfg, store, inner: Mutex::new(inner), kick: Notify::new(), said: Mutex::new(HashMap::new()), accounts: accounts::Kept::default() });
    // what was changed while starting (a credential masked in a kept message) is written now, not at the next
    // thing an agent says
    {
        let inner = app.read();
        app.save(&inner);
    }
    Ok(app)
}

/// Tools that have no hooks and are watched (a server of the tool's own): each adapter that watches is started,
/// and tells the tower what it sees the same way an event over HTTP is taken. Which of each tool's sessions are
/// open now is found by one look for all of them.
fn watch_tools(app: &Arc<App>) {
    let (app, runtime) = (app.clone(), tokio::runtime::Handle::current());
    let tower = move || {
        let (taking, runtime, holding, having) = (app.clone(), runtime.clone(), app.clone(), app.clone());
        let (finding, entering, hooking) = (app.clone(), runtime.clone(), app.clone());
        crate::adapters::Tower {
            event: Box::new(move |event| {
                let _here = runtime.enter();
                intake::record_and_take(&taking, event).is_ok()
            }),
            holds: Box::new(move |session| held::holds(&holding.read(), session)),
            found: Box::new(move |agent, idle| {
                let _here = entering.enter();
                found(&finding, agent, idle)
            }),
            // one the tower runs itself is reported by its own driver, whatever its tool lists
            hooked: Box::new(move |session| hooking.read().find(session).is_some_and(|s| s.hooked)),
            listed: Box::new(move |harness| having.read().sessions.iter().filter(|s| s.agent.harness == harness && s.listed && s.agent.deliver != Some(Deliver::Held)).map(|s| s.agent.id.clone()).collect()),
        }
    };
    crate::adapters::watch_all(&tower);
    std::thread::spawn(move || crate::adapters::find_all(tower()));
}

/// A session its tool lists as open: checked in as any agent is, and marked as listed (and, when it is new to the
/// tower and doing nothing, as waiting for him rather than working).
fn found(app: &Arc<App>, agent: Agent, idle: bool) -> bool {
    let id = agent.id.clone();
    let new = app.read().find(&id).is_none();
    let event = AgentEvent::new(agent, Event::Checkin);
    if intake::record_and_take(app, event).is_err() {
        return false;
    }
    // asked every few seconds for every open session: only one that is to be marked is changed
    if !app.read().find(&id).is_some_and(|s| !s.listed || (new && idle)) {
        return true;
    }
    let mut inner = app.change();
    if let Some(s) = inner.session(&id) {
        s.listed = true;
        s.waiting = s.waiting || (new && idle);
    }
    true
}

/// The folders his coding tools have had sessions in, read now and every minute: a new agent can be started in any
/// of them by name.
fn read_worked_folders(app: &Arc<App>) {
    let app = app.clone();
    std::thread::spawn(move || loop {
        // their records are read off the lock
        let worked = crate::adapters::worked_all();
        app.passing().worked = worked;
        std::thread::sleep(Duration::from_secs(60));
    });
}

/// What an agent's tool could not hand over while the tower was away is taken now, and whenever more is left: read
/// from the folder and taken as any event is, with no request to itself.
fn drain_spool(app: &Arc<App>) {
    let (app, runtime) = (app.clone(), tokio::runtime::Handle::current());
    std::thread::spawn(move || loop {
        crate::adapters::drain_spool(|event| {
            let _here = runtime.enter();
            intake::record_and_take(&app, event)
        });
        std::thread::sleep(Duration::from_secs(5));
    });
}

/// Every half minute: agents that have gone are let go (written only when one has), and once an hour old copies of
/// pictures and old records go, not only when the tower starts: it runs for weeks.
async fn tidy(app: Arc<App>) {
    let mut ticks = 0u32;
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        ticks += 1;
        if ticks.is_multiple_of(120) {
            tokio::task::spawn_blocking(pictures::prune);
            let app = app.clone();
            tokio::task::spawn_blocking(move || app.store.prune());
        }
        app.reap();
    }
}

/// Serve this computer's own address, then every private-network address, and the local network's when that is
/// allowed (`airband lan`), as they come up or change. Every address also takes sealed requests from a paired
/// radio that can reach it without the relay; on the local network only sealed requests are served: nothing there
/// is trusted with a token.
async fn serve(app: Arc<App>, router: Router, port: u16) -> Result<(), String> {
    let local = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| format!("cannot listen on 127.0.0.1:{port}: {e} (is airband already running?)"))?;
    log(format!("listening on 127.0.0.1:{port}"));
    let router = router.merge(relay_link::sealed_router());
    {
        let router = router.clone();
        tokio::spawn(async move { axum::serve(local, router).await });
    }
    let lan = app.cfg.lan;
    let mut bound = HashSet::new();
    loop {
        let wanted = config::overlay_addrs().into_iter().map(|a| (a, false)).chain(config::lan_addrs().into_iter().filter(|_| lan).map(|a| (a, true)));
        for (addr, sealed_only) in wanted {
            if bound.contains(&addr) {
                continue;
            }
            match tokio::net::TcpListener::bind((addr, port)).await {
                Ok(l) => {
                    log(format!("listening on {addr}:{port}{}", if sealed_only { ", for a paired radio on this network" } else { "" }));
                    bound.insert(addr);
                    let router = if sealed_only { relay_link::sealed_router() } else { router.clone() };
                    tokio::spawn(async move { axum::serve(l, router).await });
                }
                Err(e) => log(format!("cannot listen on {addr}:{port}: {e}")),
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
