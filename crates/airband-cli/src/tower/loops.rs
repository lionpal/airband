//! What the tower keeps doing on its own while it runs: noticing agents that have gone quiet, and telling him
//! how a long piece of work is going.
use super::*;

/// What each working agent is doing and what it runs on, asked of its harness's adapter every couple of seconds,
/// and only while a radio has read the feed lately: with nobody looking this costs nothing. An adapter that
/// cannot say leaves whatever the agent reported itself (an `activity` or a `model` event).
pub(super) async fn activity_loop(app: Arc<App>) {
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        // the ones at work, and any whose model is not known yet (one that has been waiting since the tower started)
        let (looked_at, everyone): (Vec<agent::Agent>, Vec<(agent::Agent, u64)>) = {
            let inner = app.read();
            if !inner.following() {
                continue;
            }
            (inner.sessions.iter().filter(|s| !s.waiting || s.model.is_none()).map(|s| s.agent.clone()).collect(), inner.sessions.iter().map(|s| (s.agent.clone(), s.told_upto)).collect())
        };
        if everyone.is_empty() {
            continue;
        }
        let (found, typed) = tokio::task::spawn_blocking(move || {
            // what he has typed to each at its own keyboard since the last look: he sees it in the chat as he goes
            let typed: Vec<_> = everyone.into_iter().map(|(a, upto)| (intake::typed_since(&a, upto), a.id, upto)).filter(|((told, _, now), _, was)| !told.is_empty() || now != was).collect();
            (looked_at.into_iter().map(|a| (crate::adapters::glance(&a), a.id)).collect::<Vec<_>>(), typed)
        })
        .await
        .unwrap_or_default();
        let mut inner = app.change();
        for ((told, pictures, upto), id, _) in typed {
            inner.told(&id, told, pictures, upto);
        }
        for (seen, id) in found {
            // looked at every couple of seconds: what it is doing is not kept, what it runs on is (written only
            // when it changed: see `Inner::changes`)
            let Some(s) = inner.session(&id) else { continue };
            if let Some(doing) = seen.doing.filter(|_| !s.waiting) {
                s.doing = Some(doing);
            }
            // what it runs on changes rarely (he switched model or effort part way): kept when it does
            if seen.model.is_some() && (s.model != seen.model || s.effort != seen.effort) {
                (s.model, s.effort) = (seen.model, seen.effort);
            }
        }
    }
}

/// An agent that works for a long time says nothing until it stops, and things he would want to know happen
/// on the way. Once a minute: for each agent that has been at work without a word for some minutes, the labels
/// of its steps since then go to the model, which writes one line if there is something in them for him and
/// nothing if there is not. The line goes in the chat as a note: no notification, not read aloud, and it does
/// not count as the agent having stopped or having spoken past a question.
pub(super) async fn progress_loop(app: Arc<App>) {
    const QUIET_FOR: u64 = 4 * 60 * 1000;
    const FEWEST: usize = 4;
    // when each agent's steps were last looked at, so a quiet stretch with nothing in it is asked about once
    // every few minutes and not every time round
    let mut looked: HashMap<String, u64> = HashMap::new();
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let due: Vec<(agent::Agent, u64)> = {
            let inner = app.read();
            let live: HashSet<&str> = inner.sessions.iter().map(|s| s.agent.id.as_str()).collect();
            looked.retain(|id, _| live.contains(id.as_str()));
            inner
                .sessions
                .iter()
                .filter(|s| !s.waiting)
                .filter_map(|s| {
                    // since it last said anything, or he last said anything to it
                    let since = inner.log.iter().rev().find(|e| e.agent.as_deref() == Some(s.agent.id.as_str()) || e.to_agents.contains(&s.agent.id)).map(|e| e.at)?;
                    let last = since.max(looked.get(&s.agent.id).copied().unwrap_or(0));
                    (now_ms().saturating_sub(last) >= QUIET_FOR).then(|| (s.agent.clone(), since))
                })
                .collect()
        };
        for (agent, since) in due {
            looked.insert(agent.id.clone(), now_ms());
            let looked_at = agent.clone();
            let steps = tokio::task::spawn_blocking(move || crate::adapters::steps(&looked_at, since)).await.unwrap_or_default();
            if steps.len() < FEWEST {
                continue;
            }
            let Some(note) = judge::progress(&app.cfg, &steps).await else { continue };
            let mut inner = app.change();
            // it spoke for itself while the model was reading: its own words are the news
            let Some(s) = inner.find(&agent.id).filter(|s| !s.waiting).cloned() else { continue };
            if inner.log.iter().any(|e| e.agent.as_deref() == Some(agent.id.as_str()) && e.at > since) {
                continue;
            }
            log(format!("{}: a note on the way ({} steps)", s.callsign.text(), steps.len()));
            inner.record(Entry { short: Some(note.clone()), ..Entry::said(config::random_hex(8), &s, Some(Kind::Fyi), note) });
            inner.tell();
        }
    }
}
