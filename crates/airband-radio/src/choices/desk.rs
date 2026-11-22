//! What a host asks of the desk about choices, and what is done when a computer can be reached.
use super::*;

/// Have the phone keep a key, and see that it did: a key the phone could not keep is not taken for kept, and
/// the service is not chosen with nothing to call it with.
pub(super) fn keep(host: &dyn DeskHost, provider: &str, key: &str) -> Result<(), RadioError> {
    host.keep_secret(provider.to_string(), Some(key.to_string()));
    match host.secret(provider.to_string()) {
        Some(kept) if kept == key => Ok(()),
        _ => Err(RadioError::Told { reason: "This phone could not keep the key, so nothing was changed. Try again.".into() }),
    }
}

#[uniffi::export]
impl Desk {
    /// The jobs he chooses something for, in the order to ask about them.
    pub fn choice_jobs(&self) -> Vec<ChoiceJob> {
        vec![ChoiceJob::Reads, ChoiceJob::Ears, ChoiceJob::Voice]
    }

    /// Everything he can choose for a job, in the order to show it, each with how it stands. Cheap: nothing
    /// is asked of a network.
    pub fn choices(&self, job: ChoiceJob) -> Vec<Choice> {
        self.shared.chooser.choices(job, &*self.shared.host, &self.shared.senses)
    }

    /// What is chosen for a job, in a few words, for a row of a settings list.
    pub fn chosen_label(&self, job: ChoiceJob) -> String {
        match self.choices(job).into_iter().find(|c| c.chosen) {
            Some(c) if c.state == ChoiceState::Waiting => "Waiting for your computer".into(),
            Some(c) => c.title,
            // what listens and what speaks are this device's: with none chosen, it says so
            None if job != ChoiceJob::Reads => "Nothing chosen".into(),
            None => String::new(),
        }
    }

    /// Choose `id` (one of [`Desk::choices`]) for a job. `key` is a key he has just typed for it; nothing uses
    /// the one already kept. `settings` is what else it is told. A service is tried with one real call before
    /// it is taken, and a model is fetched and made ready; with `later`, a model is only noted, to be fetched
    /// by [`Desk::finish_choices`] (a first run asks about everything before it downloads anything). Err says
    /// what is wrong. When no computer took it, what was chosen before still is; for his summaries, when some
    /// computers took it and others said no, it says which, and those that took it keep it. Blocks: call it
    /// off the main thread.
    pub fn choose_for(&self, job: ChoiceJob, id: String, key: Option<String>, settings: HashMap<String, String>, later: bool) -> Result<(), RadioError> {
        let chooser = &self.shared.chooser;
        let host = &*self.shared.host;
        let choice = self.choices(job).into_iter().find(|c| c.id == id).ok_or_else(|| told("That is not offered here."))?;
        let key = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        let set: BTreeMap<String, String> = settings.into_iter().map(|(k, v)| (k, v.trim().to_string())).filter(|(_, v)| !v.is_empty()).collect();
        let done = |r: Result<(), RadioError>| {
            chooser.rest();
            if r.is_ok() {
                chooser.apply(host, &self.shared.senses);
            }
            host.changed();
            r
        };
        // a key is kept under its provider's id, whichever of the provider's offers it was given for
        let (provider, offer) = (choice.provider.clone(), choice.offer.clone());
        // the key he gave now, else the one kept for that provider
        let the_key = || key.clone().or_else(|| host.secret(provider.clone())).unwrap_or_default();
        // A service that takes a key is asked, by this device, whether it takes this one: the request its row
        // names, which costs nothing. A key that does not pass is not kept and goes nowhere else.
        // A summary service whose key his computer keeps: that key is the computer's own, and the phone has none
        // to test or send.
        let kept_there = job == ChoiceJob::Reads && key.is_none() && host.secret(provider.clone()).is_none_or(|k| k.is_empty()) && choice.has_key;
        let row = airband_core::providers::provider(&provider).filter(|p| p.key && choice.place == ChoicePlace::Service && !kept_there);
        if let Some(p) = &row {
            let with = crate::services::service_of(&provider, &offer, &the_key(), set.clone().into_iter().collect());
            if with.key.is_empty() {
                return done(Err(told(format!("Enter your {}.", p.key_name))));
            }
            chooser.work(job, &id, "Testing the key", false);
            host.changed();
            let Some(check) = p.check.as_ref() else { return done(Err(told("This version of airband cannot test that key."))) };
            let outcome = (self.shared.key_test)(check, &with.key, &with.settings(job.job()));
            if let Some(why) = crate::keys::said(&outcome, p) {
                return done(Err(told(why)));
            }
            if outcome == (airband_core::providers::KeyCheck::Good { paid: true }) {
                host.note(crate::keys::charged(p));
            }
        }
        if job == ChoiceJob::Reads {
            if let Some(key) = &key
                && let Err(why) = keep(host, &provider, key) {
                    return done(Err(why));
                }
            let (radios, everyone) = {
                let state = self.shared.state.lock().unwrap();
                let online = state.stations.iter().enumerate().filter(|(_, s)| s.online).map(|(i, s)| (state.computer_name(i), s.id(), s.radio.clone())).collect::<Vec<_>>();
                (online, state.stations.len())
            };
            // no computer to ask: it waits for one
            if radios.is_empty() {
                return done(chooser.change(|s| s.reads = Some(Waiting { id: provider, offer, set, his: true, taken: Vec::new(), refused: Vec::new() })));
            }
            chooser.work(job, &id, "Asking your computer to switch", false);
            host.changed();
            let wanted = SetReads { kind: provider.clone(), offer: offer.clone(), key: the_key(), set: set.clone() };
            let answers = switch_all(&radios, &wanted);
            // every computer that answered said no: nothing changed anywhere
            if answers.taken.is_empty() {
                return done(Err(answers.told().expect("asked at least one, and none took it")));
            }
            // what is offered and in use is the main computer's: the one he chose, or the only one
            let main = { let state = self.shared.state.lock().unwrap(); state.main().map(|i| state.stations[i].radio.clone()) };
            let offered = main.and_then(|radio| radio.reads_offered());
            let (taken, refused) = (answers.taken.iter().map(|(_, id)| id.clone()).collect::<Vec<_>>(), answers.refused.iter().map(|(_, id, _)| id.clone()).collect::<Vec<_>>());
            let kept = chooser.change(|s| {
                // computers not reached now take it when they are; those that said no are not asked again
                s.reads = (taken.len() + refused.len() < everyone).then(|| Waiting { id: provider.clone(), offer: offer.clone(), set: set.clone(), his: true, taken: taken.clone(), refused: refused.clone() });
                if offered.is_some() {
                    s.offered = offered;
                }
            });
            // some switched and some did not: he is told which, and the ones that switched stay switched
            return done(kept.and(answers.told().map_or(Ok(()), Err)));
        }
        let pick = match choice.place {
            ChoicePlace::Service => {
                let with = crate::services::service_of(&provider, &offer, &the_key(), set.clone().into_iter().collect());
                // one with no key (a server of his own) has nothing to test but itself: one short real call
                if row.is_none() {
                    let tried = match job {
                        ChoiceJob::Voice => crate::mouth::check(&with),
                        _ => crate::ears::check(&with),
                    };
                    if let Err(why) = tried {
                        return done(Err(told(why)));
                    }
                }
                if let Some(key) = key
                    && let Err(why) = keep(host, &provider, &key) {
                        return done(Err(why));
                    }
                Pick { place: ChoicePlace::Service, id: provider, offer, set, voice: String::new() }
            }
            ChoicePlace::Phone => {
                let m = chooser.model(&id).ok_or_else(|| told("This app has no such model."))?;
                if !later
                    && let Err(why) = chooser.make_ready(job, &m, host) {
                        return done(Err(why));
                    }
                // the same model again keeps the voice he had picked for it
                let before = chooser.saved().pick(job);
                Pick { place: ChoicePlace::Phone, voice: before.filter(|b| b.place == ChoicePlace::Phone && b.id == id).map(|b| b.voice).unwrap_or_default(), id, offer: String::new(), set: BTreeMap::new() }
            }
        };
        done(chooser.change(|s| match job {
            ChoiceJob::Ears => s.ears = Some(pick),
            _ => s.voice = Some(pick),
        }))
    }

    /// A sentence in the device's own voice as it is now set, for a "hear it" button. Blocks while it is made.
    pub fn own_voice_sample(&self) -> Result<crate::mouth::VoiceSample, RadioError> {
        let host = &self.shared.host;
        self.shared.senses.mouth.sample(&|engine, folder, job, input| host.run_model(engine.into(), folder.into(), job, input))
    }

    /// Speak in this one of the chosen voice's voices from now on (a `name` from its [`Choice::voices`]). For a
    /// model it takes effect at once; a service is tried with it first. Blocks.
    pub fn choose_voice(&self, name: String) -> Result<(), RadioError> {
        let chooser = &self.shared.chooser;
        let Some(pick) = chooser.saved().pick(ChoiceJob::Voice) else { return Err(told(crate::voice::NOTHING_CHOSEN)) };
        match pick.place {
            ChoicePlace::Phone => {
                chooser.change(|s| {
                    if let Some(p) = s.voice.as_mut() {
                        p.voice = name;
                    }
                })?;
                chooser.apply(&*self.shared.host, &self.shared.senses);
                self.shared.host.changed();
                Ok(())
            }
            ChoicePlace::Service => {
                let mut set: HashMap<String, String> = pick.set.into_iter().collect();
                set.insert(crate::models::field::VOICE.into(), name);
                self.choose_for(ChoiceJob::Voice, service_id(&pick.id, &pick.offer), None, set, false)
            }
        }
    }

    /// The end of a first run: fetch and make ready each model he chose that is not on this device yet, the
    /// voice and then the listening. Err is the first thing that went wrong; what he chose stays chosen, and
    /// can be fetched again where he chooses. Blocks for as long as the downloads take.
    pub fn finish_choices(&self) -> Result<(), RadioError> {
        let chooser = &self.shared.chooser;
        let host = &*self.shared.host;
        let mut went = Ok(());
        for job in [ChoiceJob::Voice, ChoiceJob::Ears] {
            let Some(pick) = chooser.saved().pick(job).filter(|p| p.place == ChoicePlace::Phone) else { continue };
            let Some(m) = chooser.model(&pick.id) else { continue };
            went = chooser.make_ready(job, &m, host);
            if went.is_err() {
                break;
            }
        }
        chooser.rest();
        chooser.apply(host, &self.shared.senses);
        host.changed();
        went
    }

    /// What is being done about a choice right now, if anything, and how far along it is: for a first run's
    /// progress bar.
    pub fn choice_work(&self) -> Option<ChoiceState> {
        self.shared.chooser.at_work().map(|w| ChoiceState::Working { label: w.2, fraction: w.3 })
    }

    /// The coding tools he uses, by id, the one he uses most first.
    pub fn tools(&self) -> Vec<String> {
        self.shared.chooser.tools()
    }

    /// He uses these coding tools, the one he uses most first. What can write his summaries follows from them.
    /// Ids the catalogue does not have are left out; with none left, nothing changes.
    pub fn set_tools(&self, tools: Vec<String>) {
        let known: Vec<String> = tools.into_iter().filter(|t| airband_core::catalog::current().tools.iter().any(|k| k.id == *t)).collect();
        if !known.is_empty() {
            if let Err(e) = self.shared.chooser.change(|s| s.use_tools(known)) {
                self.shared.host.note(e.to_string());
            }
            self.shared.host.changed();
        }
    }

}

/// What the computers asked to read with something answered: those that switched, by name and id, and those
/// that did not, with why.
pub(crate) struct Switched {
    pub(crate) taken: Vec<(String, String)>,
    pub(crate) refused: Vec<(String, String, RadioError)>,
}
impl Switched {
    /// What he is told: nothing when every computer switched; otherwise which did and which did not, and why.
    pub(crate) fn told(&self) -> Option<RadioError> {
        let no: Vec<String> = self.refused.iter().map(|(name, _, e)| format!("{name} did not: {e}")).collect();
        match (self.taken.as_slice(), self.refused.as_slice()) {
            (_, []) => None,
            ([], [(_, _, e)]) => Some(told(e.to_string())),
            (taken, _) => {
                let yes = (!taken.is_empty()).then(|| format!("{} switched. ", taken.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")));
                Some(told(format!("{}{}", yes.unwrap_or_default(), no.join(". "))))
            }
        }
    }
}

/// Ask each computer, by name, id and radio, to read with `wanted`. Every one is asked, whatever the ones before
/// it said.
pub(crate) fn switch_all(radios: &[(String, String, Arc<Radio>)], wanted: &SetReads) -> Switched {
    let mut out = Switched { taken: Vec::new(), refused: Vec::new() };
    for (name, station, radio) in radios {
        match radio.set_reads(wanted) {
            Ok(()) => out.taken.push((name.clone(), station.clone())),
            Err(e) => out.refused.push((name.clone(), station.clone(), e)),
        }
    }
    out
}

/// A computer can be reached. What waits to write his summaries is sent to it, and it is asked what it can
/// read with and what it reads with now. Blocks: called on a thread of its own.
pub(crate) fn computer_reached(shared: &Shared, radio: &Radio) {
    let chooser = &shared.chooser;
    let (me, everyone) = {
        let state = shared.state.lock().unwrap();
        let me = state.stations.iter().find(|s| std::ptr::eq(&*s.radio, radio)).map(|s| s.id());
        (me, state.stations.iter().map(|s| s.id()).collect::<Vec<_>>())
    };
    if let (Some(waiting), Some(me)) = (chooser.saved().reads, me)
        && !waiting.answered(&me) {
        let wanted = SetReads { kind: waiting.id.clone(), offer: waiting.offer.clone(), key: shared.host.secret(waiting.id.clone()).unwrap_or_default(), set: waiting.set.clone() };
        // what changes is the choice that was sent: one he made meanwhile is left as it is
        let went = radio.set_reads(&wanted);
        let kept = chooser.change(|s| {
            let Some(now) = s.reads.as_mut().filter(|w| w.same(&waiting)) else { return };
            match &went {
                Ok(()) => now.taken.push(me.clone()),
                // this computer would not take it: it is not asked again, and he is told why. The others it
                // still goes to
                Err(RadioError::Told { .. } | RadioError::Refused) => now.refused.push(me.clone()),
                // not reached after all: it goes on waiting
                Err(_) => {}
            }
            if s.reads.as_ref().is_some_and(|w| everyone.iter().all(|id| w.answered(id))) {
                s.reads = None;
            }
        });
        if let Err(RadioError::Told { reason }) = went.and(kept) {
            shared.host.note(reason);
        }
    }
    // what can write summaries, and what does now, as the main computer says: another computer's word on it
    // does not stand in for the main one's
    let is_main = { let state = shared.state.lock().unwrap(); state.main().is_some_and(|i| std::ptr::eq(&*state.stations[i].radio, radio)) };
    if is_main
        && let Some(offered) = radio.reads_offered()
        && let Err(e) = chooser.change(|s| s.offered = Some(offered))
    {
        shared.host.note(e.to_string());
    }
    shared.host.changed();
}
