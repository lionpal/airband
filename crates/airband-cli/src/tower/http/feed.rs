//! The chat as a radio reads it: the feed, one message whole, its pictures, and the computer's status.
use super::*;
use axum::Extension;

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
/// A query with a number that is not one is refused, never read as none. `wait` is held to
/// [`wire::FeedQuery::WAIT_MOST`].
pub(super) async fn feed(State(app): State<Arc<App>>, Extension(RadioId(radio)): Extension<RadioId>, Ask(q): Ask<at::Feed>) -> Answer<at::Feed> {
    let until = Instant::now() + q.wait_for();
    // all of it, or one team's (`team`, its id), or one agent's (`agent`, its id)
    let wire::FeedQuery { team, agent, since, after, limit, roster: roster_seen, .. } = q;
    // A radio that is showing the chat has everything up to the change count it asks from: nothing up to there
    // needs a notification on that radio. One that says the chat is not in front of him is following it
    // without showing it, and is told nothing by that.
    if let Some(since) = since {
        let mut inner = app.change();
        // never past what exists: a count from another tower's chat says nothing about this one's
        let has = since.min(inner.rev);
        let call = inner.call(&radio);
        if call.presence.showing_chat() {
            call.has = call.has.max(has);
        }
    }
    // while a request for the whole chat is open, what agents say is read by it
    let _reading = (team.is_none() && agent.is_none()).then(|| {
        app.change().readers += 1;
        Reading(app.clone())
    });
    let limit = limit.unwrap_or(if since.is_some() { 200 } else { 50 }).clamp(1, 200);
    loop {
        // the lines to send, and the change count this answer is good up to
        let (hits, upto, roster_same, settled): (Vec<Entry>, u64, bool, bool) = {
            app.reap();
            let inner = app.read();
            let roster_same = roster_seen.is_none_or(|seen| seen == inner.roster_rev());
            let settled = !inner.settled_for(&radio, since).is_empty();
            let wanted = |e: &&Entry| inner.in_view(e, team.as_ref().map(|t| t.as_str()), agent.as_ref().map(|a| a.as_str()));
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
        return Ok(Json(Feed { schema: airband_core::radio::SCHEMA.into(), rev: upto, messages, roster: inner.roster_entries(), roster_rev: inner.roster_rev(), reads: { let r = judge::reader(&app.cfg); r.offer(crate::providers::Job::Summary).map(|(_, o)| o.label.clone()).unwrap_or_default() }, direct: config::lan_addrs().into_iter().filter(|_| app.cfg.lan).chain(config::overlay_addrs()).map(|a| format!("http://{a}:{}", app.cfg.port)).collect(), seq: inner.seq, open: open.into_iter().collect(), quick: airband_core::radio::quick_defaults().into_iter().chain(app.cfg.quick.iter().cloned()).collect(), station: config::station(&app.cfg), reads_failed: judge::failed(), answers: inner.settled_for(&radio, since) }));
    }
}

/// One message: its whole words, as read aloud (`form=spoken`, or `part=asked` for the question as he would be
/// asked it), as written (`form=full`), or in more detail than its short version.
pub(super) async fn message(State(app): State<Arc<App>>, Path(id): Path<String>, Ask(q): Ask<at::Message>) -> Answer<at::Message> {
    // a very long one is read as far as a few minutes go
    if q.form == Some(airband_core::radio::Form::Spoken) {
        // `part=asked` is the question as he would be asked it on a call: who is asking, its short form, and
        // the answers it offers, for a radio that reads it to him before he answers
        let inner = app.read();
        let text: Option<String> = inner.log.iter().find(|e| e.id == id).map(|e| match q.part {
            Some(airband_core::radio::Part::Asked) => {
                let short = spoken(&e.line());
                let choices = if e.options.is_empty() { String::new() } else { format!(" You can say {}, or answer in your own words.", either(&e.options)) };
                format!("{}. {short}{choices}", inner.sender_name(e))
            }
            // the whole of it, read from its blocks: no mark is said, a table goes row by row, code is passed over
            None => airband_core::prose::clip(&spoken(&e.full), 3000),
        });
        return text.map(|text| Json(wire::MessageText { text })).ok_or(No(Refusal::Gone));
    }
    if q.form == Some(airband_core::radio::Form::Full) {
        return app.read().log.iter().find(|e| e.id == id).map(|e| Json(wire::MessageText { text: e.full.clone() })).ok_or(No(Refusal::Gone));
    }
    detail_of(&app, &id).await.map(|text| Json(wire::MessageText { text })).ok_or(No(Refusal::Gone))
}

/// The next part of a picture's kept copy.
/// From where in its kept copy (`from`, from the beginning when it is not given).
pub(super) async fn picture_part(Path(id): Path<String>, Ask(wire::PartQuery { from }): Ask<at::Picture>) -> Answer<at::Picture> {
    tokio::task::spawn_blocking(move || pictures::part(&id, from)).await.ok().flatten().map(Json).ok_or(No(Refusal::Gone))
}

/// What can read for this tower, and what does now: the tower's own list, for a radio that offers the choice.
pub(super) async fn reads_offered(State(app): State<Arc<App>>) -> Answer<at::ReadsOffered> {
    Ok(Json(judge::offered(&app.cfg)))
}

/// A radio chooses what reads for this tower. Tried with a real call before it is kept.
pub(super) async fn reads(Sent(wanted): Sent<at::Reads>) -> Answer<at::Reads> {
    judge::set_reader(wanted).await.map(|reads| Json(wire::ReadsSet { reads })).map_err(no)
}

pub(super) async fn status(State(app): State<Arc<App>>) -> Json<Status> {
    Json(Status::of(&app))
}
