//! The chat as a radio reads it: the feed, one message whole, its pictures, and the computer's status.
use super::*;

/// One request for the whole chat, for as long as it is open.
pub(super) struct Reading(Arc<App>);

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
pub(super) async fn feed(State(app): State<Arc<App>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Json<Feed> {
    let radio = radio_of(&headers);
    // all of it, or one team's (`team`), or one agent's (`agent`, its id)
    let (team, agent) = (q.get("team").cloned(), q.get("agent").cloned());
    let since: Option<u64> = q.get("since").and_then(|v| v.parse().ok());
    // A radio that is showing the chat has everything up to the change count it asks from: nothing up to there
    // needs a notification on that radio. One that says the chat is not in front of him is following it
    // without showing it, and is told nothing by that.
    if let Some(since) = since {
        let mut inner = app.passing();
        // never past what exists: a count from another tower's chat says nothing about this one's
        let has = since.min(inner.rev);
        let call = inner.call(&radio);
        if call.presence.showing_chat() {
            call.has = call.has.max(has);
        }
    }
    // while a request for the whole chat is open, what agents say is read by it
    let _reading = (team.is_none() && agent.is_none()).then(|| {
        app.passing().readers += 1;
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
            let mut inner = app.passing();
            inner.reap();
            let roster_same = roster_seen.is_none_or(|seen| seen == inner.roster_rev());
            let settled = !inner.settled_for(&radio, since).is_empty();
            let wanted = |e: &&Entry| inner.in_view(e, team.as_deref(), agent.as_deref());
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
        let mut unjudged: Vec<Entry> = hits.iter().filter(|e| e.short.is_none() && !e.his).cloned().collect();
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
            let mut inner = app.change();
            for (id, j) in done {
                inner.fill(&id, &j);
                push::consider(&app, &mut inner, &id);
            }
        }
        let inner = app.read();
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
        return Json(Feed { schema: airband_core::radio::SCHEMA.into(), rev: upto, messages, roster: inner.roster_entries(), roster_rev: inner.roster_rev(), reads: { let r = judge::reader(&app.cfg); r.offer(crate::providers::Job::Summary).map(|(_, o)| o.label.clone()).unwrap_or_default() }, direct: config::lan_addrs().into_iter().filter(|_| app.cfg.lan).chain(config::overlay_addrs()).map(|a| format!("http://{a}:{}", app.cfg.port)).collect(), colours: inner.colours.clone(), seq: inner.seq, projects: inner.teams().into_keys().collect(), open: open.into_iter().collect(), quick: airband_core::radio::quick_defaults().into_iter().chain(app.cfg.quick.iter().cloned()).collect(), station: config::station(&app.cfg), reads_failed: judge::failed(), answers: inner.settled_for(&radio, since) });
    }
}

/// Which form of a message a radio asks for.
#[derive(Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Form {
    /// The whole message as it is read aloud, for the radio to read in its own voice.
    Spoken,
    /// The whole message as written.
    Full,
    /// What it holds that he should be able to read whole.
    Detail,
}

/// Which part of a spoken message.
#[derive(Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Part {
    /// The question as he would be asked it on a call.
    Asked,
}

#[derive(Deserialize)]
pub(super) struct MessageAsk {
    form: Option<Form>,
    part: Option<Part>,
}

pub(super) async fn message(State(app): State<Arc<App>>, Path(id): Path<String>, Query(q): Query<MessageAsk>) -> Response {
    // a very long one is read as far as a few minutes go
    if q.form == Some(Form::Spoken) {
        // `part=asked` is the question as he would be asked it on a call: who is asking, its short form, and
        // the answers it offers, for a radio that reads it to him before he answers
        let inner = app.read();
        let text: Option<String> = inner.log.iter().find(|e| e.id == id).map(|e| match q.part {
            Some(Part::Asked) => {
                let short = e.short.clone().unwrap_or_else(|| spoken(&e.full).chars().take(600).collect());
                let choices = match e.options.as_slice() {
                    [] => String::new(),
                    [one] => format!(" You can say {one}, or answer in your own words."),
                    [first @ .., last] => format!(" You can say {}, or {last}, or answer in your own words.", first.join(", ")),
                };
                format!("{}. {short}{choices}", inner.sender(e).map(|s| s.callsign.text()).unwrap_or_else(|| e.from.clone()))
            }
            // the whole of it, read from its blocks: no mark is said, a table goes row by row, code is passed over
            None => spoken(&e.full).chars().take(3000).collect(),
        });
        return match text {
            Some(text) => Json(json!({ "text": text })).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        };
    }
    if q.form == Some(Form::Full) {
        return match app.read().log.iter().find(|e| e.id == id) {
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
pub(super) async fn picture_part(Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let from = q.get("from").and_then(|v| v.parse().ok()).unwrap_or(0);
    match tokio::task::spawn_blocking(move || pictures::part(&id, from)).await.ok().flatten() {
        Some(part) => Json(part).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// What can read for this tower, and what does now: the tower's own list, for a radio that offers the choice.
pub(super) async fn reads_offered(State(app): State<Arc<App>>) -> Json<airband_core::radio::ReadsOffered> {
    Json(judge::offered(&app.cfg))
}

/// A radio chooses what reads for this tower. Tried with a real call before it is kept.
pub(super) async fn reads(Json(wanted): Json<airband_core::radio::SetReads>) -> Json<Value> {
    Json(match judge::set_reader(wanted).await {
        Ok(name) => json!({ "reads": name }),
        Err(why) => json!({ "error": why }),
    })
}

pub(super) async fn status(State(app): State<Arc<App>>) -> Json<Value> {
    let mut inner = app.passing();
    inner.reap();
    let callsign_of = |id: &str| inner.sessions.iter().find(|s| s.agent.id == id).map(|s| s.callsign.text());
    let open = inner.open_ids();
    Json(json!({
        "phone": inner.connected(),
        "held": inner.calls.values().any(|c| c.held),
        "radios": inner.calls.iter().filter(|(_, c)| c.connected()).map(|(id, _)| id.clone()).collect::<Vec<_>>(),
        "answering": inner.answering.len(),
        "projects": inner.launchable(),
        "kept": { "messages": app.store.counts().0, "events": app.store.counts().1 },
        "sessions": inner.sessions.iter().map(|s| json!({
            "callsign": s.callsign.text(), "harness": s.agent.harness, "cwd": s.agent.cwd,
            "state": inner.state_of(s, &open), "reachable": s.agent.deliver.is_some(), "meta": s.agent.meta,
        })).collect::<Vec<_>>(),
        "pending": inner.unheard().map(|e| json!({ "callsign": e.agent.as_deref().and_then(callsign_of).unwrap_or_else(|| e.from.clone()), "kind": e.kind, "say": e.short, "aloud": e.aloud })).collect::<Vec<_>>(),
        // the last changes to what is read out, each with what it was, what it became and why
        "aloud": inner.aloud_trail,
    }))
}
