//! The tower's HTTP surface: `airband.agent/1` in, `airband.radio/1` both ways, and a status page for the
//! command line. Handlers are thin: they lock, call into the other modules, and answer.
use super::*;

/// Which radio is asking.
fn radio_of(headers: &HeaderMap) -> String {
    headers.get(RADIO_HEADER).and_then(|v| v.to_str().ok()).map(|v| v.chars().filter(|c| c.is_ascii_alphanumeric() || "-_ .".contains(*c)).take(40).collect::<String>()).filter(|v| !v.is_empty()).unwrap_or_else(|| "radio".into())
}

/// Compare every byte whatever the outcome, so timing says nothing about a token.
fn same(given: &[u8], want: &[u8]) -> bool {
    given.len() == want.len() && given.iter().zip(want).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// Two tokens. The tower's own opens everything. A radio's opens `/v1/radio/` only: it must not be able to
/// report as an agent, because an agent can name a command for the tower to run.
async fn auth(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let given = req.headers().get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("").as_bytes();
    let own = same(given, app.cfg.token.as_bytes());
    let radio = same(given, app.cfg.radio_token.as_bytes()) && req.uri().path().starts_with("/v1/radio/");
    if own || radio { next.run(req).await } else { (StatusCode::UNAUTHORIZED, Json(json!({ "error": "bad token" }))).into_response() }
}

/// Every request from a radio says where the radio stands: on a call or not, showing the chat or not. It is
/// written down before the request is run, so every rule about whether he is there reads what the radio said.
async fn presence_said(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    if req.uri().path().starts_with("/v1/radio/") {
        if let Some(said) = req.headers().get(PRESENCE_HEADER).and_then(|v| v.to_str().ok()).and_then(|v| serde_json::from_str::<airband_core::radio::Presence>(v).ok()) {
            app.inner.lock().unwrap().radio_says(&radio_of(req.headers()), said);
        }
    }
    next.run(req).await
}

/// An agent's event. It is written down exactly as it arrived before anything is made of it, so an event this
/// build cannot read is still on record for one that can.
async fn events(State(app): State<Arc<App>>, body: String) -> Response {
    let Ok(raw) = serde_json::from_str::<Value>(&body) else { return (StatusCode::BAD_REQUEST, Json(json!({ "error": "not JSON" }))).into_response() };
    app.store.event(&raw);
    let ev: AgentEvent = match serde_json::from_value(raw) {
        Ok(ev) => ev,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("not an airband.agent/1 event: {e}") }))).into_response(),
    };
    // an event with a picture is slow to take in (the picture is shrunk); the other requests must not wait behind it
    match tokio::task::block_in_place(|| take_event(&app, ev)) {
        Ok(()) => Json(json!({})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

/// Take in one event of an agent's, and set going what follows from it. The one way in, for an event that
/// came over HTTP and for one a watcher inside the tower saw. Call it where a runtime can be reached.
pub(super) fn take_event(app: &Arc<App>, ev: AgentEvent) -> Result<(), String> {
    // on some of an agent's reports the tower looks at whether it is in the right team
    let reported = matches!(ev.event, Event::Said { .. }).then(|| ev.agent.id.clone());
    let spoke = matches!(ev.event, Event::Said { .. } | Event::Notice { .. }).then(|| ev.agent.id.clone());
    // its turn is over, with or without words
    let ended = matches!(ev.event, Event::Said { .. } | Event::Stopped { .. }).then(|| ev.agent.id.clone());
    on_event(app, ev)?;
    // With a phone that can be woken, or one with a banner to keep current, and nobody reading the chat,
    // what it said is read now: whether it deserves a notification, and whether it is a report he has not
    // seen, is not known until then. With a radio reading, that radio's own request
    // has it read.
    if let Some(id) = spoke.filter(|_| app.push.any() || app.banner.any()) {
        let unread: Vec<Entry> = {
            let inner = app.inner.lock().unwrap();
            inner.log.iter().rev().take(8).filter(|e| inner.readers == 0 && e.short.is_none() && e.agent.as_deref() == Some(id.as_str())).take(1).cloned().collect()
        };
        judge_later(app, unread);
    }
    if let Some(id) = &ended {
        // one the tower runs has stopped: what he said to it meanwhile goes to it now
        if app.inner.lock().unwrap().find(id).is_some_and(|s| !s.queue.is_empty()) {
            let (app, id) = (app.clone(), id.clone());
            tokio::spawn(async move { held::flush(&app, &id).await });
        }
    }
    if let Some(id) = reported.filter(|id| app.inner.lock().unwrap().find(id).is_some_and(|s| place::due(s.reports))) {
        tokio::spawn(place::look(app.clone(), id));
    }
    Ok(())
}

/// One part of a file he is attaching. Answers with how much of the file is whole from its start.
async fn attachment(Json(part): Json<airband_core::radio::AttachmentPart>) -> Response {
    match tokio::task::spawn_blocking(move || attachments::take(part)).await {
        Ok(Ok(next)) => Json(json!({ "next": next })).into_response(),
        Ok(Err(why)) => Json(json!({ "error": why })).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// The accounts the harnesses here can run under, and how full each is. `fresh=1` asks again now.
async fn accounts_list(State(app): State<Arc<App>>, Query(q): Query<HashMap<String, String>>) -> Json<airband_core::radio::Accounts> {
    Json(accounts::list(&app, q.contains_key("fresh")).await)
}
/// Switch a harness's sessions to another account. Answers with the accounts as they now stand, or with
/// `{ "error": words for him }` when it could not be done.
async fn accounts_use(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::UseAccount>) -> Json<Value> {
    Json(match accounts::switch(&app, to).await {
        Ok(now) => serde_json::to_value(now).unwrap_or_default(),
        Err(why) => json!({ "error": why }),
    })
}

/// He moved an agent to another team.
async fn team(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::MoveAgent>) -> Json<Value> {
    Json(match place::by_him(&app, &to.agent, &to.team) {
        Ok((callsign, team)) => json!({ "callsign": callsign, "team": team }),
        Err(why) => json!({ "error": why }),
    })
}

/// A folder that is his home folder or inside it, as it really is on disk (links followed). Nothing else on
/// this computer is shown to a radio or worked in at its asking.
fn inside_home(path: &str) -> Result<std::path::PathBuf, String> {
    let home = config::home().canonicalize().map_err(|e| e.to_string())?;
    let at = if path.trim().is_empty() { home.clone() } else { std::path::Path::new(path).canonicalize().map_err(|_| "That folder is not there.".to_string())? };
    if !at.starts_with(&home) || !at.is_dir() {
        return Err("Only folders inside your home folder can be used.".into());
    }
    Ok(at)
}

/// The folders inside one of his, for choosing where a new team works.
async fn folders(Json(ask): Json<airband_core::radio::FoldersAsk>) -> Json<Value> {
    let listed = inside_home(&ask.under).and_then(|at| {
        let home = config::home().canonicalize().map_err(|e| e.to_string())?;
        let mut inside: Vec<String> = std::fs::read_dir(&at).map_err(|e| e.to_string())?.flatten().filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().into_string().ok()).filter(|n| !n.starts_with('.')).collect();
        inside.sort_by_key(|n| n.to_lowercase());
        let shown = match at.strip_prefix(&home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => at.display().to_string(),
        };
        Ok(airband_core::radio::Folders { path: at.display().to_string(), shown, up: (at != home).then(|| at.parent().map(|p| p.display().to_string())).flatten(), folders: inside })
    });
    Json(match listed {
        Ok(f) => serde_json::to_value(f).unwrap(),
        Err(why) => json!({ "error": why }),
    })
}

/// He made a new team from a radio: a name, and the folder its agents work in.
async fn project(State(app): State<Arc<App>>, Json(new): Json<airband_core::radio::NewProject>) -> Json<Value> {
    let name = new.name.trim().to_lowercase();
    // a team's name is part of what its agents are called, and is said aloud: short, and plain
    if name.is_empty() || name.chars().count() > 24 || !name.chars().all(|c| c.is_alphanumeric() || c == '-') {
        return Json(json!({ "error": "A team\u{2019}s name is one word: letters, numbers and dashes." }));
    }
    let folder = if new.make {
        let wanted = std::path::PathBuf::from(&new.folder);
        match wanted.parent().map(|p| inside_home(&p.display().to_string())) {
            Some(Ok(parent)) => match wanted.file_name().filter(|n| !n.to_string_lossy().starts_with('.')) {
                Some(last) => {
                    let made = parent.join(last);
                    if let Err(e) = std::fs::create_dir(&made) {
                        if !made.is_dir() {
                            return Json(json!({ "error": format!("That folder could not be made: {e}") }));
                        }
                    }
                    Ok(made)
                }
                None => Err("That is not a name for a folder.".to_string()),
            },
            Some(Err(why)) => Err(why),
            None => Err("That is not a folder.".to_string()),
        }
    } else {
        inside_home(&new.folder)
    };
    let folder = match folder {
        Ok(f) => f.display().to_string(),
        Err(why) => return Json(json!({ "error": why })),
    };
    {
        let mut inner = app.inner.lock().unwrap();
        if inner.launchable(&app.cfg).get(&name).is_some_and(|has| *has != folder) {
            return Json(json!({ "error": format!("There is already a team called {name}.") }));
        }
        inner.projects.insert(name.clone(), folder.clone());
        app.save(&inner);
    }
    log(format!("a radio made the team {name}, in {folder}"));
    // its first agent, with nothing to do yet, when the tower runs agents itself: the team is then there to see
    let held = held::launcher_for(&app, new.tool.as_deref()).is_some_and(|(_, l)| l.held.is_some());
    if !held {
        return Json(json!({ "say": format!("{name} is ready. Say launch {name}, then what it should do.") }));
    }
    let reply = launch::launch(&app, &name, &Idea { tool: new.tool.clone(), project: Some(name.clone()), model: None, effort: None, task: String::new() }).await;
    Json(json!({ "say": reply.say }))
}

/// He asked the tower to take over an agent he started in a terminal.
async fn takeover(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::TakeOver>) -> Json<Value> {
    Json(match held::take_over(&app, &to.agent).await {
        Ok(()) => json!({}),
        Err(why) => json!({ "error": why }),
    })
}

/// He stopped what an agent is doing.
async fn stop_agent(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::StopAgent>) -> Json<Value> {
    Json(match held::stop(&app, &to.agent).await {
        Ok(()) => json!({}),
        Err(why) => json!({ "error": why }),
    })
}
/// He ended an agent the tower runs.
async fn end_agent(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::EndAgent>) -> Json<Value> {
    let held = app.inner.lock().unwrap().find(&to.agent).map(|s| s.agent.deliver == Some(Deliver::Held));
    Json(match held {
        None => json!({ "error": "That agent is no longer running." }),
        Some(false) => json!({ "error": "Only an agent airband runs can be ended from here." }),
        Some(true) if held::end(&app, &to.agent).await => json!({}),
        Some(true) => json!({ "error": "It had already ended." }),
    })
}
/// A radio says which names his other computers use.
async fn taken(State(app): State<Arc<App>>, Json(t): Json<airband_core::radio::Taken>) -> Json<Value> {
    let mut inner = app.inner.lock().unwrap();
    if inner.taken_elsewhere(&t.callsigns) {
        app.save(&inner);
        app.kick.notify_one();
    }
    Json(json!({}))
}
/// He let a question go without answering it.
async fn dismiss(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::Dismiss>) -> Json<Value> {
    let mut inner = app.inner.lock().unwrap();
    let Some(e) = inner.log.iter_mut().find(|e| e.id == to.message) else { return Json(json!({ "error": "That message is gone." })) };
    e.dismissed = true;
    // it is not read out either
    if e.aloud.unheard() {
        inner.set_aloud(&to.message, Aloud::dropped(Dropped::Dismissed));
    }
    inner.touch(&to.message);
    app.save(&inner);
    app.kick.notify_one();
    Json(json!({}))
}
/// He took back something he said that was still waiting its turn.
async fn unsend(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::Unsend>) -> Json<Value> {
    Json(match held::unsend(&app, &to.message) {
        Ok(()) => json!({}),
        Err(why) => json!({ "error": why }),
    })
}

/// End an agent the tower runs itself (`airband end <id>`).
async fn held_end(State(app): State<Arc<App>>, Path(id): Path<String>) -> StatusCode {
    if held::end(&app, &id).await { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND }
}

/// A harness asks whether its agent may do something. The answer is fetched separately, so nothing is held open
/// across a tower restart.
/// A radio says where it can be woken when it is not listening (its address at a push service), or takes that back.
async fn push_target(State(app): State<Arc<App>>, headers: HeaderMap, Json(target): Json<airband_core::radio::PushTarget>) -> Json<Value> {
    push::register(&app, &radio_of(&headers), target);
    Json(json!({}))
}

/// A radio says which coding tools he uses, the one he uses most first. Only tools this computer can start
/// are kept.
async fn uses_tools(State(app): State<Arc<App>>, Json(said): Json<airband_core::radio::UsesTools>) -> Json<Value> {
    let known = app.cfg.tools();
    let tools: Vec<String> = said.tools.into_iter().filter(|t| known.contains(t)).collect();
    let mut inner = app.inner.lock().unwrap();
    if inner.tools != tools {
        inner.tools = tools;
        app.save(&inner);
    }
    Json(json!({}))
}

/// A radio says where its banner can be put up or changed while the app is asleep, or takes that back.
async fn banner_target(State(app): State<Arc<App>>, headers: HeaderMap, Json(target): Json<airband_core::radio::BannerTarget>) -> Json<Value> {
    banner::register(&app, &radio_of(&headers), target);
    Json(json!({}))
}

async fn permission_ask(State(app): State<Arc<App>>, Json(ask): Json<airband_core::agent::PermissionAsk>) -> Json<airband_core::agent::PermissionState> {
    Json(permit::ask(&app, ask))
}
async fn permission_state(State(app): State<Arc<App>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Json<airband_core::agent::PermissionState> {
    Json(permit::state(&app, &id, q.get("wait").and_then(|v| v.parse().ok()).unwrap_or(0)).await)
}
async fn permission_withdraw(State(app): State<Arc<App>>, Path(id): Path<String>) -> Json<Value> {
    permit::withdraw(&app, &id);
    Json(json!({}))
}

async fn next(State(app): State<Arc<App>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let radio = radio_of(&headers);
    {
        let mut inner = app.inner.lock().unwrap();
        // He has just turned reading aloud on, and the radio says so (`joined`). What was said long before is
        // in the chat for him to read: it is not read out now, hours late. A radio that only went quiet for a
        // while and is back says no such thing, and nothing is taken from the queue for it.
        if q.contains_key("joined") {
            inner.leave_old_news();
            inner.call(&radio).began = super::now();
        }
        // The radio says which transmission it last got (`have`). If the tower gave it another since and has
        // heard nothing about it, the answer that carried it was lost on the way: it is given again now, not
        // after two minutes of silence.
        let call = inner.call(&radio);
        if let (Some(have), Some(lost)) = (q.get("have"), call.handed.clone()) {
            if *have != lost.id && call.outbox.is_none() && !call.ready {
                log(format!("{radio} never got what it was handed ({}); handing it over again", lost.callsign));
                call.outbox = Some(lost);
            }
        }
    }
    app.kick.notify_one();
    let until = Instant::now() + HOLD;
    while Instant::now() < until {
        {
            let mut inner = app.inner.lock().unwrap();
            let call = inner.call(&radio);
            if let Some(t) = call.outbox.take() {
                call.handed = Some(t.clone());
                return Json(t).into_response();
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    StatusCode::NO_CONTENT.into_response()
}

/// A radio was handed something to read out and no sound came of it. It goes back in the queue, the reason is
/// written down, and that radio is handed nothing more for a minute.
async fn unheard(State(app): State<Arc<App>>, headers: HeaderMap, Json(u): Json<Value>) -> Json<Value> {
    let radio = radio_of(&headers);
    let mut inner = app.inner.lock().unwrap();
    let id = u["id"].as_str().unwrap_or_default();
    let who = inner.log.iter().find(|e| e.id == id).map(|e| e.from.clone()).unwrap_or_default();
    log(format!("{radio} could not read {who} aloud ({}); it stays queued (line {id}), and nothing more goes to that radio for a minute", u["why"].as_str().unwrap_or("no reason given")));
    if inner.log.iter().any(|e| e.id == id && matches!(&e.aloud, Aloud::Handed { radio: r, .. } if *r == radio)) {
        inner.set_aloud(id, Aloud::Waiting);
    }
    let call = inner.call(&radio);
    call.quiet_until = Some(Instant::now() + Duration::from_secs(60));
    call.ready = true;
    call.outbox = None;
    call.handed = None;
    app.save(&inner);
    Json(json!({}))
}

/// A radio says where it stands, when that changes and while it has nothing else to ask (he has the
/// microphone open on a call): so the tower does not take it for gone.
async fn presence(State(app): State<Arc<App>>, headers: HeaderMap, Json(said): Json<airband_core::radio::Presence>) -> Json<Value> {
    app.inner.lock().unwrap().radio_says(&radio_of(&headers), said);
    app.kick.notify_one();
    Json(json!({}))
}

async fn heard(State(app): State<Arc<App>>, headers: HeaderMap, Json(h): Json<Heard>) -> Json<Value> {
    let radio = radio_of(&headers);
    let mut inner = app.inner.lock().unwrap();
    // remember when he was last told, so an unanswered question comes round again later and not at once
    let times = inner.offered.get(&h.id).map(|(n, _)| *n).unwrap_or(0);
    inner.offered.insert(h.id.clone(), (times + 1, Instant::now()));
    let call = inner.call(&radio);
    if call.handed.as_ref().is_some_and(|t| t.id == h.id) {
        call.handed = None;
    }
    // a reminder of a question he has heard before is not in the queue; what was is now heard
    if let Some(agent_id) = inner.log.iter().find(|e| e.id == h.id && e.aloud.unheard()).map(|e| e.agent.clone().unwrap_or_default()) {
        inner.set_aloud(&h.id, Aloud::Heard { radio: radio.clone(), at: super::now() });
        if let Some(s) = inner.session(&agent_id) {
            s.last_heard = Some(h.id.clone());
            inner.call(&radio).focus = Some(agent_id);
        }
        app.save(&inner);
    }
    Json(json!({}))
}

async fn ready(State(app): State<Arc<App>>, headers: HeaderMap) -> Json<Value> {
    let waiting = {
        let mut inner = app.inner.lock().unwrap();
        let radio = radio_of(&headers);
        let call = inner.call(&radio);
        call.ready = true;
        // ready for the next with no word about the last (he turned reading off part way, or its own voice
        // failed): the last is still to be heard
        inner.give_back(&radio);
        inner.unheard().count()
    };
    app.kick.notify_one();
    Json(json!({ "waiting": waiting }))
}

/// An utterance that takes longer than this to answer is written down as slow.
const SLOW: Duration = Duration::from_secs(10);

/// Something he said. It is answered at once: the tower has it, under its id. Working out who it is for and
/// handing it over can take a model a while, and no request waits on that: what comes of it is told in the
/// feed, and to anyone who asks again under the same id.
async fn utterance(State(app): State<Arc<App>>, headers: HeaderMap, Json(u): Json<Utterance>) -> Response {
    use airband_core::radio::Outcome;
    let radio = radio_of(&headers);
    // how it came: through the relay, straight over his own network, kept by the relay while the tower was
    // away, or from this computer
    let via = headers.get(VIA_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("this computer").to_string();
    if u.id.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "what he said has to come with an id" }))).into_response();
    }
    {
        // the id is taken before anything is done, under the one lock, so the same utterance arriving twice
        // (a retry, or two ways at once) is taken once, whether or not the tower restarted in between
        let mut inner = app.inner.lock().unwrap();
        if inner.answering.contains(&u.id) {
            log(format!("utterance {} from {radio} by {via}: asked after, still being answered", u.id));
            return Json(Outcome::Taken).into_response();
        }
        match app.store.begin_receipt(&u.id) {
            Receipt::Done(reply) => {
                log(format!("utterance {} from {radio} by {via}: already answered, the same answer given again", u.id));
                return Json(Outcome::Answered { reply }).into_response();
            }
            Receipt::Begun => {
                inner.answering.insert(u.id.clone());
            }
        }
    }
    // answered in the order it came from this radio: several can arrive in one moment (what the relay kept
    // while the tower was away), and each is finished before the next is begun
    let line = app.said.lock().unwrap().entry(radio.clone()).or_insert_with(|| {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>>();
        tokio::spawn(async move {
            while let Some(next) = rx.recv().await {
                next.await;
            }
        });
        tx
    }).clone();
    let app_ = app.clone();
    let _ = line.send(Box::pin(async move {
        let app = app_;
        let began = Instant::now();
        // a file with no words still says something: look at it
        let text = match (u.text.trim(), u.attachments.is_empty()) {
            ("", true) => None,
            ("", false) => Some("Look at what I attached."),
            (said, _) => Some(said),
        };
        let reply = match text {
            Some(text) => on_utterance(&app, &radio, Some(&u.id), text, u.re.as_deref(), u.scope.as_deref(), u.label.as_deref(), &u.attachments, &u.others).await,
            None => Reply::default(),
        };
        app.store.finish_receipt(&u.id, &reply);
        // one line for everything he says: its id (the same on the phone and at the relay), how it came, what
        // became of it and how long that took
        let took = began.elapsed();
        log(format!("utterance {} from {radio} by {via}: answered in {:.1} s{}{}", u.id, took.as_secs_f32(), if reply.say.is_empty() { String::new() } else { format!(" (\"{}\")", reply.say.chars().take(80).collect::<String>()) }, if took > SLOW { ", which is slow" } else { "" }));
        let mut inner = app.inner.lock().unwrap();
        inner.answering.remove(&u.id);
        inner.settle(&radio, &u.id, Outcome::Answered { reply });
    }));
    Json(Outcome::Taken).into_response()
}

/// How many short versions one feed request waits for. The rest are made behind it and arrive as changes.
const JUDGE_NOW: usize = 12;
/// How many are made at once behind a request.
const JUDGE_AT_ONCE: usize = 3;

/// Make the short version of these logged messages, a few at a time, and mark each changed as it is done so
/// every radio following the feed is sent it. Nothing is left unjudged because a radio was away when it came in.
fn judge_later(app: &Arc<App>, entries: Vec<Entry>) {
    let entries: Vec<Entry> = {
        let mut inner = app.inner.lock().unwrap();
        entries.into_iter().filter(|e| inner.judging.insert(e.id.clone())).collect()
    };
    if entries.is_empty() {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let limit = Arc::new(tokio::sync::Semaphore::new(JUDGE_AT_ONCE));
        let mut tasks = tokio::task::JoinSet::new();
        for e in entries {
            let (app, limit) = (app.clone(), limit.clone());
            tasks.spawn(async move {
                let _slot = limit.acquire().await;
                let j = judge_entry(&app, e.clone()).await;
                let mut inner = app.inner.lock().unwrap();
                inner.fill(&e.id, j.kind, &j.say, &j.options, j.recommended, j.read.as_deref(), j.background, j.own_session);
                push::consider(&app, &mut inner, &e.id);
                app.save(&inner);
            });
        }
        tasks.join_all().await;
    });
}

async fn judge_entry(app: &Arc<App>, e: Entry) -> Judged {
    let item = Item { say: None, ..app.inner.lock().unwrap().item(&e) };
    prepare(&app.cfg, &item).await
}

/// One request for the whole chat, for as long as it is open.
struct Reading(Arc<App>);
impl Drop for Reading {
    fn drop(&mut self) {
        let mut inner = self.0.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.readers = inner.readers.saturating_sub(1);
    }
}

/// Chat mode: the log, each message cut to a line or two.
///
/// `since=<rev>` is how a radio follows it: everything new or changed after that change count, oldest change
/// first, and the count to pass next time. A message changes when its short version is made or its sender is
/// renamed, so a radio always ends up with the current form of every line. `since=0` is a radio starting from
/// nothing: it gets the newest lines.
/// With no `since`, `after=<seq>` gives the lines added after that position, and never a change to an earlier one.
async fn feed(State(app): State<Arc<App>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Json<Feed> {
    let radio = radio_of(&headers);
    let scope = q.get("scope").cloned().unwrap_or_else(|| "all".into());
    let since: Option<u64> = q.get("since").and_then(|v| v.parse().ok());
    // A radio that is showing the chat has everything up to the change count it asks from: nothing up to there
    // needs a notification on that radio. One that says the chat is not in front of him is following it
    // without showing it, and is told nothing by that.
    if let Some(since) = since {
        let mut inner = app.inner.lock().unwrap();
        // never past what exists: a count from another tower's chat says nothing about this one's
        let has = since.min(inner.rev);
        let call = inner.call(&radio);
        if call.presence.showing_chat() {
            call.has = call.has.max(has);
        }
    }
    // while a request for the whole chat is open, what agents say is read by it
    let _reading = (scope == "all").then(|| {
        app.inner.lock().unwrap().readers += 1;
        Reading(app.clone())
    });
    let after: u64 = q.get("after").and_then(|v| v.parse().ok()).unwrap_or(0);
    let limit: usize = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(if since.is_some() { 200 } else { 50 }).clamp(1, 200);
    let until = Instant::now() + Duration::from_secs(q.get("wait").and_then(|v| v.parse().ok()).unwrap_or(0).min(25));
    // the roster this radio last saw, if it says: then a change to the roster alone also ends the wait
    let roster_seen: Option<u64> = q.get("roster").and_then(|v| v.parse().ok());
    loop {
        // the lines to send, and the change count this answer is good up to
        let (hits, upto, roster_same, settled): (Vec<Entry>, u64, bool, bool) = {
            let mut inner = app.inner.lock().unwrap();
            inner.reap();
            let roster_same = roster_seen.is_none_or(|seen| seen == inner.roster_rev());
            let settled = !inner.settled_for(&radio, since).is_empty();
            let wanted = |e: &&Entry| in_scope(&scope, &inner.message(e));
            match since {
                Some(0) | None => {
                    let hits: Vec<&Entry> = inner.log.iter().filter(|e| e.seq > after).filter(wanted).collect();
                    (hits[hits.len().saturating_sub(limit)..].iter().map(|e| (*e).clone()).collect(), inner.rev, roster_same, settled)
                }
                Some(since) => {
                    let mut hits: Vec<&Entry> = inner.log.iter().filter(|e| e.rev > since).filter(wanted).collect();
                    hits.sort_by_key(|e| e.rev);
                    // more changed than fits: send the earliest, and say only how far that goes
                    let upto = if hits.len() > limit { hits[limit - 1].rev } else { inner.rev };
                    (hits.into_iter().take(limit).cloned().collect(), upto, roster_same, settled)
                }
            }
        };
        if hits.is_empty() && roster_same && !settled && Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(300)).await;
            continue;
        }
        // Someone is reading, so the short versions are worth making. This request waits for the ones that
        // matter most (each agent's latest, then the newest); the rest are made behind it.
        let mut unjudged: Vec<Entry> = hits.iter().filter(|e| e.short.is_none() && e.from != "you").cloned().collect();
        if !unjudged.is_empty() {
            let latest: HashSet<String> = {
                let mut by_agent: HashMap<Option<String>, String> = HashMap::new();
                unjudged.iter().for_each(|e| {
                    by_agent.insert(e.agent.clone(), e.id.clone());
                });
                by_agent.into_values().collect()
            };
            unjudged.sort_by_key(|e| (!latest.contains(&e.id), std::cmp::Reverse(e.seq)));
            let later = unjudged.split_off(unjudged.len().min(JUDGE_NOW));
            judge_later(&app, later);
            let mut tasks = tokio::task::JoinSet::new();
            for e in unjudged {
                let app = app.clone();
                tasks.spawn(async move { (e.id.clone(), judge_entry(&app, e).await) });
            }
            let done = tasks.join_all().await;
            let mut inner = app.inner.lock().unwrap();
            for (id, j) in done {
                inner.fill(&id, j.kind, &j.say, &j.options, j.recommended, j.read.as_deref(), j.background, j.own_session);
                push::consider(&app, &mut inner, &id);
            }
            app.save(&inner);
        }
        let inner = app.inner.lock().unwrap();
        // if the only changes since this answer was started are to the lines in it (their short versions were
        // just made), it is good up to now, and the radio is not sent the same lines again next time
        let sent: HashSet<&String> = hits.iter().map(|e| &e.id).collect();
        let whole = hits.len() < limit || matches!(since, Some(0) | None);
        let upto = if whole && inner.log.iter().filter(|e| e.rev > upto).all(|e| sent.contains(&e.id)) { inner.rev } else { upto };
        let open = inner.open_ids();
        let messages: Vec<Message> = hits
            .iter()
            .map(|hit| {
                // as it is now: it may just have been given its short version
                let mut m = inner.message(inner.log.iter().find(|e| e.id == hit.id).unwrap_or(hit));
                m.open = open.contains(&m.id);
                m
            })
            .collect();
        return Json(Feed { schema: airband_core::radio::SCHEMA.into(), rev: upto, messages, roster: inner.roster_entries(), roster_rev: inner.roster_rev(), reads: { let r = judge::reader(&app.cfg); r.offer(crate::providers::Job::Summary).map(|(_, o)| o.label.clone()).unwrap_or_default() }, direct: config::lan_addrs().into_iter().filter(|_| app.cfg.lan).chain(config::tailscale_addrs()).map(|a| format!("http://{a}:{}", app.cfg.port)).collect(), colours: inner.colours.clone(), seq: inner.seq, projects: inner.launchable(&app.cfg).into_keys().collect(), open: open.into_iter().collect(), quick: airband_core::radio::quick_defaults().into_iter().chain(app.cfg.quick.iter().cloned()).collect(), station: config::station(&app.cfg), reads_failed: judge::failed(), answers: inner.settled_for(&radio, since) });
    }
}

async fn message(State(app): State<Arc<App>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    // `spoken` is the whole message as it is read aloud, for the radio to read in its own voice. A very long
    // one is read as far as a few minutes go
    if q.get("form").map(String::as_str) == Some("spoken") {
        // `part=asked` is the question as he would be asked it on a call: who is asking, its short form, and
        // the answers it offers, for a radio that reads it to him before he answers
        let text: Option<String> = app.inner.lock().unwrap().log.iter().find(|e| e.id == id).map(|e| match q.get("part").map(String::as_str) {
            Some("asked") => {
                let short = e.short.clone().unwrap_or_else(|| spoken(&e.full).chars().take(600).collect());
                let choices = match e.options.as_slice() {
                    [] => String::new(),
                    [one] => format!(" You can say {one}, or answer in your own words."),
                    [first @ .., last] => format!(" You can say {}, or {last}, or answer in your own words.", first.join(", ")),
                };
                format!("{}. {short}{choices}", airband_core::routing::shown(&e.from))
            }
            // the whole of it, read from its blocks: no mark is said, a table goes row by row, code is passed over
            _ => airband_radio::prose::said(&e.full).chars().take(3000).collect(),
        });
        return match text {
            Some(text) => Json(json!({ "text": text })).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        };
    }
    if q.get("form").map(String::as_str) == Some("full") {
        return match app.inner.lock().unwrap().log.iter().find(|e| e.id == id) {
            Some(e) => Json(More { text: e.full.clone() }).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        };
    }
    match detail_of(&app, &id).await {
        Some(text) => Json(More { text }).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The next part of a picture's kept copy.
async fn picture_part(Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let from = q.get("from").and_then(|v| v.parse().ok()).unwrap_or(0);
    match tokio::task::spawn_blocking(move || pictures::part(&id, from)).await.ok().flatten() {
        Some(part) => Json(part).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// What can read for this tower, and what does now: the tower's own list, for a radio that offers the choice.
async fn reads_offered(State(app): State<Arc<App>>) -> Json<airband_core::radio::ReadsOffered> {
    Json(judge::offered(&app.cfg))
}

/// A radio chooses what reads for this tower. Tried with a real call before it is kept.
async fn reads(Json(wanted): Json<airband_core::radio::SetReads>) -> Json<Value> {
    Json(match judge::set_reader(wanted).await {
        Ok(name) => json!({ "reads": name }),
        Err(why) => json!({ "error": why }),
    })
}

async fn status(State(app): State<Arc<App>>) -> Json<Value> {
    let mut inner = app.inner.lock().unwrap();
    inner.reap();
    let callsign_of = |id: &str| inner.sessions.iter().find(|s| s.agent.id == id).map(|s| s.callsign.text());
    Json(json!({
        "phone": inner.connected(),
        "held": inner.calls.values().any(|c| c.held),
        "radios": inner.calls.iter().filter(|(_, c)| c.connected()).map(|(id, _)| id.clone()).collect::<Vec<_>>(),
        "answering": inner.answering.len(),
        "projects": inner.launchable(&app.cfg),
        "kept": { "messages": app.store.counts().0, "events": app.store.counts().1 },
        "sessions": inner.sessions.iter().map(|s| json!({
            "callsign": s.callsign.text(), "harness": s.agent.harness, "cwd": s.agent.cwd,
            "state": if s.waiting { "waiting" } else { "working" }, "reachable": s.agent.deliver.is_some(), "meta": s.agent.meta,
        })).collect::<Vec<_>>(),
        "pending": inner.unheard().map(|e| json!({ "callsign": e.agent.as_deref().and_then(callsign_of).unwrap_or_else(|| e.from.clone()), "kind": e.kind, "say": e.short, "aloud": e.aloud })).collect::<Vec<_>>(),
        // the last changes to what is read out, each with what it was, what it became and why
        "aloud": inner.aloud_trail,
    }))
}

/// `airband triage`: judge one message from standard input the way the tower would, and print the verdict.
/// For trying the judgement on real messages.

/// Every route the tower serves. The relay link hands sealed requests to this same router, so a request is
/// handled identically however it arrived.
pub(super) fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/v1/agent/events", post(events))
        .route("/v1/agent/held/{id}", axum::routing::delete(held_end))
        .route("/v1/agent/permission", post(permission_ask))
        .route("/v1/agent/permission/{id}", get(permission_state).delete(permission_withdraw))
        .route("/v1/radio/next", get(next))
        .route("/v1/radio/heard", post(heard))
        .route("/v1/radio/presence", post(presence))
        .route("/v1/radio/unheard", post(unheard))
        .route("/v1/radio/ready", post(ready))
        .route("/v1/radio/utterance", post(utterance))
        .route("/v1/radio/feed", get(feed))
        .route("/v1/radio/picture/{id}", get(picture_part))
        .route("/v1/radio/reads", get(reads_offered).post(reads))
        .route("/v1/radio/message/{id}", get(message))
        .route("/v1/radio/attachment", post(attachment))
        .route("/v1/radio/accounts", get(accounts_list))
        .route("/v1/radio/accounts/use", post(accounts_use))
        .route("/v1/radio/team", post(team))
        .route("/v1/radio/folders", post(folders))
        .route("/v1/radio/project", post(project))
        .route("/v1/radio/takeover", post(takeover))
        .route("/v1/radio/stop", post(stop_agent))
        .route("/v1/radio/unsend", post(unsend))
        .route("/v1/radio/dismiss", post(dismiss))
        .route("/v1/radio/taken", post(taken))
        .route("/v1/radio/end", post(end_agent))
        .route("/v1/radio/push", post(push_target))
        .route("/v1/radio/tools", post(uses_tools))
        .route("/v1/radio/banner", post(banner_target))
        .route("/v1/status", get(status))
        .layer(middleware::from_fn_with_state(app.clone(), presence_said))
        .layer(middleware::from_fn_with_state(app.clone(), auth))
        .with_state(app)
}
