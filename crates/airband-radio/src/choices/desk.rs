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
            if let Some(why) = outcome.said(&p.name) {
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
            let wanted = ReadsWanted { id: provider.clone(), offer: offer.clone(), set: set.clone(), his: true };
            let phone_key = host.secret(provider.clone()).filter(|k| !k.is_empty());
            let (online, everyone) = {
                let state = self.shared.state.lock().unwrap();
                (state.computers.iter().filter(|s| s.online).map(|s| (s.id(), s.radio.clone())).collect::<Vec<_>>(), state.computers.iter().map(|s| s.id()).collect::<Vec<_>>())
            };
            if !online.is_empty() {
                chooser.work(job, &id, "Asking your computer to switch", false);
                host.changed();
            }
            // every computer it can reach is asked at once, each with what it needs: a key only where it has none
            let per = fan_out(online, |radio| match wanted.body_for(phone_key.as_deref(), radio.reads_offered().as_ref()) {
                Ok(body) => Reach::of(radio.set_reads(&body)),
                Err(why) => Reach::Skipped(why),
            });
            // every computer it reached said no: nothing changed anywhere, and he is told why
            if !per.is_empty() && per.values().all(|r| r.why_not().is_some()) {
                let state = self.shared.state.lock().unwrap();
                let why: Vec<String> = per.iter().map(|(c, r)| if per.len() == 1 { r.why_not().unwrap_or_default().to_string() } else { format!("{}: {}", state.computer_name(c), r.why_not().unwrap_or_default()) }).collect();
                return done(Err(told(why.join(" "))));
            }
            // what is offered and in use is the main computer's: the one he chose, or the only one
            let main = { let state = self.shared.state.lock().unwrap(); state.main().map(|s| s.radio.clone()) };
            let offered = main.and_then(|radio| radio.reads_offered());
            // each computer's answer is kept: one that said no is shown with why and not asked again, one not
            // reached is sent it when it is, and the ones that took it keep it
            return done(chooser.change(|s| {
                let mut intent = Intent::after(s.reads.as_ref(), wanted.clone());
                per.iter().for_each(|(c, r)| intent.record(c, r));
                // with no computer paired yet it waits for the first
                s.reads = (everyone.is_empty() || !intent.done_by_all(&everyone)).then_some(intent);
                if offered.is_some() {
                    s.offered = offered;
                }
            }));
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

/// A computer can be reached. What waits to write his summaries is sent to it, and it is asked what it can
/// read with and what it reads with now. Blocks: called on a thread of its own.
pub(crate) fn computer_reached(shared: &Shared, radio: &Radio) {
    let chooser = &shared.chooser;
    let (me, everyone) = {
        let state = shared.state.lock().unwrap();
        let me = state.computers.iter().find(|s| std::ptr::eq(&*s.radio, radio)).map(|s| s.id());
        (me, state.computers.iter().map(|s| s.id()).collect::<Vec<_>>())
    };
    if let (Some(intent), Some(me)) = (chooser.saved().reads, me)
        && intent.owed(&me) {
        let phone_key = shared.host.secret(intent.wanted.id.clone()).filter(|k| !k.is_empty());
        let reach = match intent.wanted.body_for(phone_key.as_deref(), radio.reads_offered().as_ref()) {
            Ok(body) => Reach::of(radio.set_reads(&body)),
            Err(why) => Reach::Skipped(why),
        };
        // what changes is the choice that was sent: one he made meanwhile is left as it is
        let kept = chooser.change(|s| {
            let Some(now) = s.reads.as_mut().filter(|i| i.rev == intent.rev) else { return };
            now.record(&me, &reach);
            if now.done_by_all(&everyone) {
                s.reads = None;
            }
        });
        if let Err(RadioError::Told { reason }) = kept {
            shared.host.note(reason);
        }
    }
    // what can write summaries, and what does now, as the main computer says: another computer's word on it
    // does not stand in for the main one's
    let is_main = { let state = shared.state.lock().unwrap(); state.main().is_some_and(|s| std::ptr::eq(&*s.radio, radio)) };
    if is_main
        && let Some(offered) = radio.reads_offered()
        && let Err(e) = chooser.change(|s| s.offered = Some(offered))
    {
        shared.host.note(e.to_string());
    }
    shared.host.changed();
}
