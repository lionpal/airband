//! What a host asks of the desk about choices, and what is done when a computer can be reached.
use super::*;

/// Have the phone keep a key, and see that it did: a key the phone could not keep is not taken for kept, and
/// the service is not chosen with nothing to call it with.
pub(super) fn keep(host: &dyn DeskHost, provider: &str, key: &str) -> Result<(), RadioError> {
    host.keep_secret(provider.to_string(), Some(key.to_string()));
    match host.secret(provider.to_string()) {
        Some(kept) if kept == key => Ok(()),
        _ => Err(told("This phone could not keep the key, so nothing was changed. Try again.")),
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
        let paired: Vec<StationId> = self.shared.state.lock().unwrap().computers.iter().map(|c| c.id()).collect();
        self.shared.chooser.choices(job, &*self.shared.host, &self.shared.senses, &paired)
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
        let the_key = || key.clone().or_else(|| kept_key(host, &provider)).unwrap_or_default();
        // A service that takes a key is asked, by this device, whether it takes this one: the request its row
        // names, which costs nothing. A key that does not pass is not kept and goes nowhere else.
        // A summary service whose key his computer keeps: that key is the computer's own, and the phone has none
        // to test or send.
        let kept_there = job == ChoiceJob::Reads && key.is_none() && kept_key(host, &provider).is_none() && choice.has_key;
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
            let phone_key = kept_key(host, &provider);
            let (online, everyone, main) = {
                let state = self.shared.state.lock().unwrap();
                let online = state.computers.iter().filter(|s| s.online).map(|s| (s.id(), s.radio.clone())).collect::<Vec<_>>();
                (online, state.computers.iter().map(|s| s.id()).collect::<Vec<_>>(), state.main().map(|s| s.id()))
            };
            if !online.is_empty() {
                chooser.work(job, &id, "Asking your computer to switch", false);
                host.changed();
            }
            // every computer it can reach is asked at once, each with what it needs: a key only where it has none
            let per = fan_out(online, |radio| match radio.reads_offered() {
                // he is choosing now: nothing newer can be waiting
                Ok(offered) => send_reads(&wanted, phone_key.as_deref(), radio, offered, || true).unwrap_or(Reach::Unreachable),
                Err(e) => Reach::of(Err(e)),
            });
            // every computer it reached said no: nothing changed anywhere, and he is told why
            if !per.is_empty() && per.values().all(|r| r.why_not().is_some()) {
                let state = self.shared.state.lock().unwrap();
                let why: Vec<String> = per.iter().map(|(c, r)| if per.len() == 1 { r.why_not().unwrap_or_default().to_string() } else { format!("{}: {}", state.computer_name(c), r.why_not().unwrap_or_default()) }).collect();
                return done(Err(told(why.join(" "))));
            }
            // what is offered and in use is the main computer's: the one he chose, or the only one
            let offered = main.and_then(|m| match per.get(&m) { Some(Reach::Done(o)) => Some(o.clone()), _ => None });
            // each computer's answer is kept: one that said no is shown with why and not asked again, one not
            // reached is sent it when it is, and the ones that took it keep it. It is kept after every computer
            // has it too, for a computer paired later.
            return done(chooser.change(|s| {
                per.iter().for_each(|(c, r)| if let Reach::Done(o) = r { s.heard_from(c, o) });
                let mut intent = Intent::after(s.reads.as_ref(), wanted.clone());
                everyone.iter().for_each(|c| intent.record(c, per.get(c).unwrap_or(&Reach::Unreachable)));
                s.reads = Some(intent);
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
    /// Ids the catalogue does not have are left out; with none left, nothing changes, so one always stays.
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

/// What one computer does with what is to write his summaries, given what it said it can read with: sent the
/// choice, with a key only where it has none of its own, unless `still` says the choice has changed since. When
/// it takes it, its own account comes back, with what it reads now as it took it.
fn send_reads(wanted: &ReadsWanted, phone_key: Option<&str>, radio: &Radio, mut offered: ReadsOffered, still: impl Fn() -> bool) -> Option<Reach<ReadsOffered>> {
    let body = match wanted.body_for(phone_key, &offered) {
        Ok(body) => body,
        Err(why) => return Some(Reach::Skipped(why)),
    };
    // a choice he made while it was being asked goes in its own round: this older one is not sent
    if !still() {
        return None;
    }
    Some(Reach::of(radio.set_reads(&body)).map(|()| {
        // it tried it with a real call before it said yes: that is what it reads with now, and it works
        offered.now = Some(airband_core::radio::OfferName { provider: wanted.id.clone(), offer: wanted.offer.clone() });
        offered.failed = None;
        offered
    }))
}

/// A computer can be reached, on its connection `epoch`: what waits to write his summaries is sent to it if it
/// is owed it, and the main computer's word on what it can read with is kept. It is asked what it can read with
/// once. One that could not say is asked again later: nothing is decided about it from what it did not say.
/// Blocks: called on a thread of its own, when it is reached and on each round while it is owed the choice.
pub(crate) fn tell_reads(shared: &Shared, station: &StationId, epoch: u64, ask_offered: bool) {
    let chooser = &shared.chooser;
    let (radio, is_main) = {
        let mut state = shared.state.lock().unwrap();
        let is_main = state.main().is_some_and(|s| s.id() == *station);
        let Some(computer) = state.live(station, epoch) else { return };
        (computer.radio.clone(), is_main)
    };
    let owed = chooser.saved().reads.filter(|i| i.owed(station));
    if owed.is_none() && !ask_offered {
        return;
    }
    // what the main computer said can read is taken only if nothing newer was kept while this was asked
    let asked_at = chooser.saved().offered;
    let asked = radio.reads_offered();
    if let Ok(offered) = &asked
        && let Err(e) = chooser.change(|s| s.heard_from(station, offered))
    {
        shared.host.note(e.said());
    }
    let offered = match (owed, asked) {
        (Some(intent), Ok(offered)) => {
            let phone_key = kept_key(&*shared.host, &intent.wanted.id);
            let still = || chooser.saved().reads.is_some_and(|i| i.rev == intent.rev);
            let Some(reach) = send_reads(&intent.wanted, phone_key.as_deref(), &radio, offered.clone(), still) else { return };
            // what changes is the choice that was sent: one he made meanwhile is left as it is
            let kept = chooser.change(|s| {
                if let Some(now) = s.reads.as_mut().filter(|i| i.rev == intent.rev) {
                    now.record(station, &reach);
                }
            });
            if let Err(e) = kept {
                shared.host.note(e.said());
            }
            match reach {
                Reach::Done(after) => after,
                _ => offered,
            }
        }
        (Some(intent), Err(e)) => {
            let reach: Reach<ReadsOffered> = Reach::of(Err(e));
            let kept = chooser.change(|s| {
                if let Some(now) = s.reads.as_mut().filter(|i| i.rev == intent.rev) {
                    now.record(station, &reach);
                }
            });
            if let Err(e) = kept {
                shared.host.note(e.said());
            }
            return;
        }
        (None, Ok(offered)) => offered,
        // it could not say: nothing is decided from that, and it is asked again when it is reached
        (None, Err(_)) => return,
    };
    // what can write summaries, and what does now, as the main computer says: another computer's word on it
    // does not stand in for the main one's, and a computer no longer the main one, or on a connection since
    // replaced, is not taken at its word
    let still_main = || {
        let mut state = shared.state.lock().unwrap();
        state.main().is_some_and(|s| s.id() == *station) && state.live(station, epoch).is_some()
    };
    if is_main
        && still_main()
        && let Err(e) = chooser.change(|s| {
            if s.offered == asked_at {
                s.offered = Some(offered);
            }
        })
    {
        shared.host.note(e.said());
    }
    shared.host.changed();
}
