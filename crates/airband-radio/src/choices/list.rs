//! The one list for each job, made from what he chose and the state of things outside.
use super::*;
use crate::desk::Standing;

/// What a list is made from besides what he chose: the state of things outside, gathered once.
pub(super) struct Facts<'a> {
    /// The models this device can run.
    pub(super) models: Vec<Arc<Model>>,
    /// For a model's kind: is it on this device, and is that the version the catalogue names?
    pub(super) here: &'a dyn Fn(&Model) -> (bool, bool),
    /// Is a key kept on this device for this provider?
    pub(super) has_key: &'a dyn Fn(&str) -> bool,
    /// Why what is chosen for this job did not work the last time, if it did not.
    pub(super) failed: Option<String>,
    pub(super) work: Option<(String, String, Option<f64>)>,
    /// The computers paired now, by id.
    pub(super) paired: &'a [StationId],
    /// The model this job starts with, when this device cannot run it, and why.
    pub(super) cannot: Option<(Arc<Model>, crate::engines::CannotRun)>,
}

pub(super) fn megabytes(m: &Model) -> u32 {
    (m.files.iter().map(|f| f.1).sum::<u64>() as f64 / 1e6).round() as u32
}

/// The line under a service's name: what it needs from him and what it costs.
pub(super) fn line(s: &Service, has_key: bool) -> String {
    if !s.key {
        return s.about.clone();
    }
    let needs = if has_key { format!("Using your {}", s.key_name) } else { format!("Enter your {}", s.key_name) };
    if s.price.is_empty() { needs } else { format!("{needs}. {}.", s.price) }
}

/// What a service is called in a list: its provider and its offer together. Never taken apart again: a
/// choice carries both beside it.
pub(super) fn service_id(provider: &str, offer: &str) -> String {
    format!("{provider}/{offer}")
}

/// The one list for a job.
pub(super) fn list(job: ChoiceJob, saved: &Saved, facts: &Facts) -> Vec<Choice> {
    let working = |id: &str| facts.work.as_ref().filter(|w| w.0 == id).map(|w| ChoiceState::Working { label: w.1.clone(), fraction: w.2 });
    let of_service = |s: Service, chosen: bool, has_key: bool, voice: String, state: ChoiceState| Choice {
        line: line(&s, has_key),
        id: service_id(&s.kind, &s.offer),
        place: ChoicePlace::Service,
        provider: s.kind,
        offer: s.offer,
        title: s.name,
        free: s.free,
        more: s.more,
        chosen,
        state,
        key: s.key,
        has_key,
        key_name: s.key_name,
        keys_at: s.keys_at,
        asks: s.asks,
        voices: s.voices,
        voice,
    };
    if job == ChoiceJob::Reads {
        // his computer's own list once it has said; until then, what this build knows a computer can use
        let (now, offers): (Option<OfferName>, Vec<(Service, bool)>) = match &saved.offered {
            Some(o) => (
                o.now.clone(),
                o.offers
                    .iter()
                    .map(|r| (Service::from(r), r.has_key))
                    .collect(),
            ),
            None => (None, crate::services::services(Job::Summary).into_iter().map(|s| (s, false)).collect()),
        };
        // what reads now could not, the last time his computer asked it: it stays chosen and says why
        let reads_failed = saved.offered.as_ref().and_then(|o| o.failed);
        let chosen = saved.reads.as_ref().map(|w| OfferName { provider: w.wanted.id.clone(), offer: w.wanted.offer.clone() }).or(now);
        // what uses the sign-in of a coding tool is offered to someone who uses that tool, those first
        let (mine, rest): (Vec<_>, Vec<_>) = offers.into_iter().filter(|(s, _)| s.login.is_empty() || saved.tools.contains(&s.login)).partition(|(s, _)| !s.login.is_empty());
        return mine
            .into_iter()
            .chain(rest)
            .map(|(s, there)| {
                let is = chosen.as_ref().is_some_and(|c| c.provider == s.kind && c.offer == s.offer);
                // he is asked for a key only when some paired computer has said it has none of its own
                let has_key = (facts.has_key)(&s.kind) || (there && !saved.key_wanted(facts.paired, &s.kind));
                let state = working(&service_id(&s.kind, &s.offer)).unwrap_or(match () {
                    // no computer has it yet: it waits, or every computer it was put to said no, and why
                    _ if is && let Some(w) = &saved.reads && w.standing(facts.paired) != Standing::Taken => match w.standing(facts.paired) {
                        Standing::Refused(why) => ChoiceState::Failed { why: why.join(" ") },
                        _ => ChoiceState::Waiting,
                    },
                    _ if is && reads_failed.is_some() => ChoiceState::Failed { why: reads_failed.map(|t| t.said(&s.name)).unwrap_or_default() },
                    _ if s.key && !has_key => ChoiceState::NeedsKey,
                    _ => ChoiceState::Ready,
                });
                of_service(s, is, has_key, String::new(), state)
            })
            .collect();
    }
    let pick = saved.pick(job);
    let failed = |chosen: bool| facts.failed.clone().filter(|_| chosen).map(|why| ChoiceState::Failed { why });
    let mut out: Vec<Choice> = Vec::new();
    for m in facts.models.iter().filter(|m| m.job == job.job()) {
        let m = &**m;
        let (fetched, current) = (facts.here)(m);
        let chosen = pick.as_ref().is_some_and(|p| p.place == ChoicePlace::Phone && p.id == m.kind);
        let size = megabytes(m);
        let state = working(&m.kind).or(failed(chosen)).unwrap_or(match (fetched, current) {
            (false, _) => ChoiceState::NeedsDownload { megabytes: size },
            (true, false) => ChoiceState::Stale { megabytes: size },
            (true, true) => ChoiceState::Ready,
        });
        out.push(Choice {
            id: m.kind.clone(),
            place: ChoicePlace::Phone,
            provider: m.kind.clone(),
            offer: String::new(),
            title: m.name.clone(),
            line: if fetched { "Runs on this phone".into() } else { format!("Runs on this phone. {size}\u{a0}MB download") },
            free: true,
            more: false,
            chosen,
            state,
            key: false,
            has_key: false,
            key_name: String::new(),
            keys_at: String::new(),
            asks: Vec::new(),
            voices: m.voices.iter().map(|(name, about)| VoiceChoice { name: name.clone(), label: name.clone(), about: about.clone(), preview: format!("{}/previews/{name}.m4a", m.base) }).collect(),
            // with none named, the voice the row names as its usual one
            voice: m.voice(pick.as_ref().filter(|_| chosen).map(|p| p.voice.as_str()).unwrap_or_default()).to_string(),
        });
    }
    // the model this job starts with, when this device cannot run it: shown, with why, so he knows to pick
    // something else; never chosen, and nothing is chosen for him in its place
    if let Some((m, why)) = &facts.cannot {
        out.push(Choice {
            id: m.kind.clone(),
            place: ChoicePlace::Phone,
            provider: m.kind.clone(),
            offer: String::new(),
            title: m.name.clone(),
            line: why.said().into(),
            free: true,
            more: false,
            chosen: false,
            state: ChoiceState::CannotRun { why: why.said().into() },
            key: false,
            has_key: false,
            key_name: String::new(),
            keys_at: String::new(),
            asks: Vec::new(),
            voices: Vec::new(),
            voice: String::new(),
        });
    }
    for s in crate::services::services(job.job()) {
        let chosen = pick.as_ref().is_some_and(|p| p.place == ChoicePlace::Service && p.id == s.kind && p.offer == s.offer);
        let has_key = (facts.has_key)(&s.kind);
        let state = working(&service_id(&s.kind, &s.offer)).or(failed(chosen)).unwrap_or(if s.key && !has_key { ChoiceState::NeedsKey } else { ChoiceState::Ready });
        // with none named, the usual answer the row gives for its voice
        let voice = pick.as_ref().filter(|_| chosen).and_then(|p| p.set.get(crate::models::field::VOICE).cloned()).filter(|v| !v.is_empty()).unwrap_or_else(|| s.usual_voice.clone());
        out.push(of_service(s, chosen, has_key, voice, state));
    }
    out
}
