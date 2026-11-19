//! The tower's HTTP surface: `airband.agent/1` in, `airband.radio/1` both ways, and a status page for the
//! command line. Handlers are thin: they lock, call into the other modules, and answer.
use super::*;
use airband_core::radio::EarsGrant;

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

/// An agent's event. It is written down exactly as it arrived before anything is made of it, so an event this
/// build cannot read is still on record for one that can.
async fn events(State(app): State<Arc<App>>, body: String) -> Response {
    let Ok(raw) = serde_json::from_str::<Value>(&body) else { return (StatusCode::BAD_REQUEST, Json(json!({ "error": "not JSON" }))).into_response() };
    app.store.event(&raw);
    let ev: AgentEvent = match serde_json::from_value(raw) {
        Ok(ev) => ev,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("not an airband.agent/1 event: {e}") }))).into_response(),
    };
    // on some of an agent's reports the tower looks at whether it is in the right team
    let reported = matches!(ev.event, Event::Said { .. }).then(|| ev.agent.id.clone());
    // an event with a picture is slow to take in (the picture is shrunk); the other requests must not wait behind it
    match tokio::task::block_in_place(|| on_event(&app, ev)) {
        Ok(()) => {
            if let Some(id) = &reported {
                // one the tower runs has stopped: what he said to it meanwhile goes to it now
                if app.inner.lock().unwrap().find(id).is_some_and(|s| !s.queue.is_empty()) {
                    let (app, id) = (app.clone(), id.clone());
                    tokio::spawn(async move { held::flush(&app, &id).await });
                }
            }
            if let Some(id) = reported.filter(|id| app.inner.lock().unwrap().find(id).is_some_and(|s| place::due(s.reports))) {
                tokio::spawn(place::look(app.clone(), id));
            }
            Json(json!({})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

/// One part of a file he is attaching. Answers with where the next part starts.
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
/// He took back something he said that was still waiting its turn.
async fn unsend(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::Unsend>) -> Json<Value> {
    Json(match held::unsend(&app, &to.message) {
        Ok(()) => json!({}),
        Err(why) => json!({ "error": why }),
    })
}

/// End an agent the tower runs itself (`airband end <id>`).
async fn held_end(State(app): State<Arc<App>>, Path(id): Path<String>) -> StatusCode {
    if held::end(&app, &id) { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND }
}

/// A harness asks whether its agent may do something. The answer is fetched separately, so nothing is held open
/// across a tower restart.
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

async fn next(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let radio = radio_of(&headers);
    {
        let mut inner = app.inner.lock().unwrap();
        if !inner.call(&radio).connected() {
            log(format!("{radio} connected"));
            // whatever it was in the middle of last time, it is free now
            let call = inner.call(&radio);
            call.ready = true;
            call.outbox = None;
            inner.reap();
        }
        inner.call(&radio).last_poll = Some(Instant::now());
    }
    app.kick.notify_one();
    let until = Instant::now() + HOLD;
    while Instant::now() < until {
        {
            let mut inner = app.inner.lock().unwrap();
            let call = inner.call(&radio);
            call.last_poll = Some(Instant::now());
            if let Some(t) = call.outbox.take() {
                return Json(t).into_response();
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn heard(State(app): State<Arc<App>>, headers: HeaderMap, Json(h): Json<Heard>) -> Json<Value> {
    let radio = radio_of(&headers);
    let mut inner = app.inner.lock().unwrap();
    // remember when he was last told, so an unanswered question comes round again later and not at once
    let times = inner.offered.get(&h.id).map(|(n, _)| *n).unwrap_or(0);
    inner.offered.insert(h.id.clone(), (times + 1, Instant::now()));
    if let Some(i) = inner.pending.iter().position(|p| p.id == h.id) {
        let item = inner.pending.remove(i);
        let agent_id = item.agent_id.clone();
        if let Some(s) = inner.session(&agent_id) {
            s.last = Some(item);
            inner.call(&radio).focus = Some(agent_id);
        }
        app.save(&inner);
    }
    Json(json!({}))
}

async fn ready(State(app): State<Arc<App>>, headers: HeaderMap) -> Json<Value> {
    let waiting = {
        let mut inner = app.inner.lock().unwrap();
        inner.call(&radio_of(&headers)).ready = true;
        inner.pending.len()
    };
    app.kick.notify_one();
    Json(json!({ "waiting": waiting }))
}

async fn utterance(State(app): State<Arc<App>>, headers: HeaderMap, Json(u): Json<Utterance>) -> Json<Reply> {
    let radio = radio_of(&headers);
    // a file with no words still says something: look at it
    let text = match (u.text.trim(), u.attachments.is_empty()) {
        ("", true) => return Json(Reply::default()),
        ("", false) => "Look at what I attached.",
        (said, _) => said,
    };
    // take the id before doing anything, so a retry of the same utterance is answered once, however often it
    // arrives and whether or not the tower restarted in between
    if let Some(id) = &u.id {
        match app.store.begin_receipt(id) {
            Receipt::Begun => {}
            Receipt::Done(reply) => return Json(reply),
            Receipt::InProgress => {
                for _ in 0..120 {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    if let Some(reply) = app.store.receipt_reply(id) {
                        return Json(reply);
                    }
                }
                return Json(Reply { say: "Still working on that.".into(), listen: false, ..Reply::default() });
            }
        }
    }
    let mut reply = on_utterance(&app, &radio, u.id.as_deref(), text, u.re.as_deref(), u.scope.as_deref(), u.label.as_deref(), &u.attachments, &u.others).await;
    // on a call the tower's answer is spoken too: start its voice now, and the radio fetches it as it is made
    if !reply.say.is_empty() && app.inner.lock().unwrap().call(&radio).connected() {
        reply.speech = app.speech.start(&app.voice(), &reply.say);
    }
    if let Some(id) = &u.id {
        app.store.finish_receipt(id, &reply);
    }
    Json(reply)
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
                inner.fill(&e.id, j.kind, &j.say, &j.options, j.recommended, j.read.as_deref());
                app.save(&inner);
            });
        }
        tasks.join_all().await;
    });
}

async fn judge_entry(app: &Arc<App>, e: Entry) -> Judged {
    let notice = e.full.starts_with("Needs you at the keyboard: ");
    let asked = e.agent.as_deref().and_then(|a| app.inner.lock().unwrap().last_instruction(a));
    prepare(&app.cfg, &Item { id: e.id, agent_id: e.agent.unwrap_or_default(), notice, full: e.full, kind: e.kind, say: None, detail: None, created: e.at, asked }).await
}

/// Chat mode: the log, each message cut to a line or two.
///
/// `since=<rev>` is how a radio follows it: everything new or changed after that change count, oldest change
/// first, and the count to pass next time. A message changes when its short version is made or its sender is
/// renamed, so a radio always ends up with the current form of every line. `since=0` is a radio starting from
/// nothing: it gets the newest lines.
/// `after=<seq>` is the older form, still answered: lines added after that position, never changes to earlier ones.
async fn feed(State(app): State<Arc<App>>, Query(q): Query<HashMap<String, String>>) -> Json<Feed> {
    let scope = q.get("scope").cloned().unwrap_or_else(|| "all".into());
    let since: Option<u64> = q.get("since").and_then(|v| v.parse().ok());
    let after: u64 = q.get("after").and_then(|v| v.parse().ok()).unwrap_or(0);
    let limit: usize = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(if since.is_some() { 200 } else { 50 }).clamp(1, 200);
    let until = Instant::now() + Duration::from_secs(q.get("wait").and_then(|v| v.parse().ok()).unwrap_or(0).min(25));
    // the roster this radio last saw, if it says: then a change to the roster alone also ends the wait
    let roster_seen: Option<u64> = q.get("roster").and_then(|v| v.parse().ok());
    loop {
        // the lines to send, and the change count this answer is good up to
        let (hits, upto, roster_same): (Vec<Entry>, u64, bool) = {
            let mut inner = app.inner.lock().unwrap();
            inner.reap();
            // someone is reading the feed: there is somebody to ask
            inner.last_reader = Some(Instant::now());
            let roster_same = roster_seen.is_none_or(|seen| seen == inner.roster_rev());
            let wanted = |e: &&Entry| in_scope(&scope, &inner.message(e));
            match since {
                Some(0) | None => {
                    let hits: Vec<&Entry> = inner.log.iter().filter(|e| e.seq > after).filter(wanted).collect();
                    (hits[hits.len().saturating_sub(limit)..].iter().map(|e| (*e).clone()).collect(), inner.rev, roster_same)
                }
                Some(since) => {
                    let mut hits: Vec<&Entry> = inner.log.iter().filter(|e| e.rev > since).filter(wanted).collect();
                    hits.sort_by_key(|e| e.rev);
                    // more changed than fits: send the earliest, and say only how far that goes
                    let upto = if hits.len() > limit { hits[limit - 1].rev } else { inner.rev };
                    (hits.into_iter().take(limit).cloned().collect(), upto, roster_same)
                }
            }
        };
        if hits.is_empty() && roster_same && Instant::now() < until {
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
                inner.fill(&id, j.kind, &j.say, &j.options, j.recommended, j.read.as_deref());
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
        return Json(Feed { schema: airband_core::radio::SCHEMA.into(), rev: upto, messages, roster: inner.roster_entries(), roster_rev: inner.roster_rev(), colours: inner.colours.clone(), seq: inner.seq, projects: inner.launchable(&app.cfg).into_keys().collect(), open: open.into_iter().collect(), quick: airband_core::radio::quick_defaults().into_iter().chain(app.cfg.quick.iter().cloned()).collect(), station: config::station(&app.cfg) });
    }
}

async fn message(State(app): State<Arc<App>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    // the whole message, read aloud in the tower's voice: the id of its speech. A very long one is read as far
    // as a few minutes go
    if q.get("form").map(String::as_str) == Some("speech") {
        let text: Option<String> = app.inner.lock().unwrap().log.iter().find(|e| e.id == id).map(|e| spoken(&e.full).chars().take(3000).collect());
        return match text.and_then(|text| app.speech.start(&app.voice(), &text)) {
            Some(speech) => Json(json!({ "speech": speech })).into_response(),
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

/// The next part of a clip, as far as the voice has got.
async fn speech_part(State(app): State<Arc<App>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let from = q.get("from").and_then(|v| v.parse().ok()).unwrap_or(0);
    match app.speech.part(&id, from).await {
        Some(part) => Json(part).into_response(),
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

/// What should listen on the phone: its own recognition, or a service it reaches with a short-lived pass.
async fn ears(State(app): State<Arc<App>>) -> Json<EarsGrant> {
    Json(ears::grant(&app).await)
}

async fn status(State(app): State<Arc<App>>) -> Json<Value> {
    let mut inner = app.inner.lock().unwrap();
    inner.reap();
    let callsign_of = |id: &str| inner.sessions.iter().find(|s| s.agent.id == id).map(|s| s.callsign.text());
    Json(json!({
        "phone": inner.connected(),
        "held": inner.calls.values().any(|c| c.held),
        "radios": inner.calls.iter().filter(|(_, c)| c.connected()).map(|(id, _)| id.clone()).collect::<Vec<_>>(),
        "projects": inner.launchable(&app.cfg),
        "kept": { "messages": app.store.counts().0, "events": app.store.counts().1 },
        "sessions": inner.sessions.iter().map(|s| json!({
            "callsign": s.callsign.text(), "harness": s.agent.harness, "cwd": s.agent.cwd,
            "state": if s.waiting { "waiting" } else { "working" }, "reachable": s.agent.deliver.is_some(), "meta": s.agent.meta,
        })).collect::<Vec<_>>(),
        "pending": inner.pending.iter().map(|p| json!({ "callsign": callsign_of(&p.agent_id), "kind": p.kind, "say": p.say })).collect::<Vec<_>>(),
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
        .route("/v1/radio/ready", post(ready))
        .route("/v1/radio/utterance", post(utterance))
        .route("/v1/radio/feed", get(feed))
        .route("/v1/radio/speech/{id}", get(speech_part))
        .route("/v1/radio/picture/{id}", get(picture_part))
        .route("/v1/radio/ears", get(ears))
        .route("/v1/radio/message/{id}", get(message))
        .route("/v1/radio/attachment", post(attachment))
        .route("/v1/radio/accounts", get(accounts_list))
        .route("/v1/radio/accounts/use", post(accounts_use))
        .route("/v1/radio/team", post(team))
        .route("/v1/radio/takeover", post(takeover))
        .route("/v1/radio/stop", post(stop_agent))
        .route("/v1/radio/unsend", post(unsend))
        .route("/v1/status", get(status))
        .layer(middleware::from_fn_with_state(app.clone(), auth))
        .with_state(app)
}
