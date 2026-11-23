//! What he picked, as it is kept in `choices.json`.
use super::*;
use crate::desk::Intent;

/// What he picked for a job done on this device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Pick {
    pub(super) place: ChoicePlace,
    /// The model's kind or the provider's id.
    #[serde(default)]
    pub(super) id: String,
    /// For a service: which of its provider's offers.
    #[serde(default)]
    pub(super) offer: String,
    /// What a service was told beyond its key.
    #[serde(default)]
    pub(super) set: BTreeMap<String, String>,
    /// Which of a model's voices speaks. Empty is its usual one.
    #[serde(default)]
    pub(super) voice: String,
}

/// Why a computer was not sent what writes his summaries: it needs a key, and neither it nor this phone has one.
pub(crate) const NO_KEY: &str = "It has no key for this. Add one here.";

/// Why a computer was not sent something, decided here: it could not take it as things are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Withheld {
    /// It needs a key that neither it nor this phone has.
    NoKey,
}

impl Withheld {
    /// In words for him.
    pub(crate) fn said(self) -> String {
        match self {
            Withheld::NoKey => NO_KEY.into(),
        }
    }
}

/// What is to write his summaries, sent to every computer ([`Intent`]): it waits for each that has not
/// answered, and is let go once every computer has it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReadsWanted {
    /// The provider's id, and which of its offers.
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) offer: String,
    #[serde(default)]
    pub(crate) set: BTreeMap<String, String>,
    /// He picked it himself. Otherwise it follows from the coding tool he said he uses most, and changes when
    /// that does.
    #[serde(default)]
    pub(crate) his: bool,
}

impl ReadsWanted {
    /// What one computer is sent: with no key when it keeps its own or the provider takes none; else with the
    /// key on this phone. Err, in words for him, when it needs a key neither has. `there` is what that
    /// computer says it can read with.
    pub(crate) fn body_for(&self, phone_key: Option<&str>, there: &ReadsOffered) -> Result<airband_core::radio::SetReads, Withheld> {
        let offer = there.offers.iter().find(|r| r.provider == self.id && r.offer == self.offer);
        let needs_key = offer.map_or_else(|| airband_core::providers::provider(&self.id).is_some_and(|p| p.key), |r| r.key);
        // a computer's own key is its own: the phone's goes only where there is none
        let key = match phone_key.filter(|k| !k.is_empty()) {
            _ if !needs_key || offer.is_some_and(|r| r.has_key) => String::new(),
            Some(key) => key.to_string(),
            None => return Err(Withheld::NoKey),
        };
        Ok(airband_core::radio::SetReads { kind: self.id.clone(), offer: self.offer.clone(), key, set: self.set.clone() })
    }
}

/// A kept pick as this build reads it. One it cannot read (a place this build does not have) is nothing
/// chosen: he is told to choose where he would use it, and the rest of what was kept stands.
pub(super) fn pick_or_nothing<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Pick>, D::Error> {
    Ok(Option::<serde_json::Value>::deserialize(d)?.and_then(|v| serde_json::from_value(v).ok()))
}

/// Everything kept about what he chose.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Saved {
    #[serde(default, deserialize_with = "pick_or_nothing")]
    pub(super) ears: Option<Pick>,
    #[serde(default, deserialize_with = "pick_or_nothing")]
    pub(super) voice: Option<Pick>,
    #[serde(default)]
    pub(super) reads: Option<Intent<ReadsWanted>>,
    /// The coding tools he uses, by id, the one he uses most first.
    #[serde(default)]
    pub(super) tools: Vec<String>,
    /// What his computer last said can read for it, and which does. Nothing until a computer has said.
    #[serde(default)]
    pub(super) offered: Option<ReadsOffered>,
    /// The providers each computer keeps a summary key of its own for, by the computer's id, as each last
    /// said. The phone asks him for a key only when some paired computer has none of its own.
    #[serde(default)]
    pub(super) kept_there: BTreeMap<StationId, Vec<String>>,
    /// What was changed while the file could not be read, to be kept over what it holds once it can be.
    #[serde(skip)]
    pub(super) touched: Touched,
}

/// Which parts of [`Saved`] were changed since the app opened. Not part of what is kept: two that differ only
/// in this are the same.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Touched {
    ears: bool,
    voice: bool,
    reads: bool,
    tools: bool,
    offered: bool,
    kept_there: bool,
}

impl PartialEq for Touched {
    fn eq(&self, _: &Touched) -> bool {
        true
    }
}

impl Touched {
    /// What differs between `before` and `after`, added to what had.
    pub(super) fn add(&mut self, before: &Saved, after: &Saved) {
        self.ears |= before.ears != after.ears;
        self.voice |= before.voice != after.voice;
        self.reads |= before.reads != after.reads;
        self.tools |= before.tools != after.tools;
        self.offered |= before.offered != after.offered;
        self.kept_there |= before.kept_there != after.kept_there;
    }
}

impl crate::kept::Rejoin for Saved {
    /// What he chose while the file could not be read is kept over what it holds; the rest is the file's.
    fn rejoin(&mut self, since: Saved) {
        let t = since.touched;
        if t.ears {
            self.ears = since.ears;
        }
        if t.voice {
            self.voice = since.voice;
        }
        if t.reads {
            self.reads = since.reads;
        }
        if t.tools {
            self.tools = since.tools;
        }
        if t.offered {
            self.offered = since.offered;
        }
        if t.kept_there {
            self.kept_there.extend(since.kept_there);
        }
    }
}

impl Saved {
    /// A new install: for listening and for the voice, the model whose row says it is the usual one
    /// (`usual` in its row) and that runs on this device, because it needs nothing from him (it is fetched when
    /// he finishes the first run). Where no row says so for a job, nothing is chosen for it. Which coding
    /// tools he uses is his to say: none is taken for his until he picks.
    pub(super) fn fresh(device: &HostDevice) -> Saved {
        let usual = |job: Job| crate::models::usual(device, job).map(|m| Pick { place: ChoicePlace::Phone, id: m.kind.clone(), offer: String::new(), set: BTreeMap::new(), voice: String::new() });
        Saved { ears: usual(Job::Ears), voice: usual(Job::Voice), ..Saved::default() }
    }

    /// Is a key for `provider` wanted from him: some paired computer has said it keeps none of its own. One
    /// that has not said yet is not taken to have none.
    pub(super) fn key_wanted(&self, paired: &[StationId], provider: &str) -> bool {
        paired.iter().any(|c| self.kept_there.get(c).is_some_and(|p| !p.iter().any(|p| p == provider)))
    }

    /// What a computer said it can read with: which providers it keeps a key of its own for.
    pub(super) fn heard_from(&mut self, computer: &StationId, offered: &ReadsOffered) {
        self.kept_there.insert(computer.clone(), offered.offers.iter().filter(|o| o.has_key).map(|o| o.provider.clone()).collect());
    }

    /// What is chosen on this device for a job, if anything is.
    pub(super) fn pick(&self, job: ChoiceJob) -> Option<Pick> {
        match job {
            ChoiceJob::Ears => self.ears.clone(),
            ChoiceJob::Voice => self.voice.clone(),
            ChoiceJob::Reads => None,
        }
    }

    /// The coding tools he uses are these. While no computer has said what it reads with and he has not
    /// picked for himself, what waits to write his summaries is the sign-in of the one he uses most.
    pub(super) fn use_tools(&mut self, tools: Vec<String>) {
        self.tools = tools;
        if self.offered.is_none() && self.reads.as_ref().is_none_or(|w| !w.wanted.his) {
            let most = self.tools.first().cloned().unwrap_or_default();
            let before = self.reads.take();
            // the offer its row marks as the one to use for that sign-in; none marked is nothing chosen
            self.reads = crate::services::services(Job::Summary).into_iter().find(|s| !most.is_empty() && s.login == most && !s.more).map(|s| Intent::after(before.as_ref(), ReadsWanted { id: s.kind, offer: s.offer, set: BTreeMap::new(), his: false }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kept::Rejoin;

    fn offered(has_key: bool) -> ReadsOffered {
        ReadsOffered { offers: vec![airband_core::radio::ReadsOffer { provider: "anthropic".into(), offer: "haiku".into(), key: true, has_key, ..Default::default() }], ..ReadsOffered::default() }
    }

    #[test]
    fn a_summary_key_is_asked_for_only_when_a_paired_computer_has_said_it_has_none() {
        // (audit 5, BUG-20)
        let (main, other, gone) = (StationId::new("main"), StationId::new("other"), StationId::new("gone"));
        let mut s = Saved::default();
        s.heard_from(&main, &offered(true));
        assert!(!s.key_wanted(&[main.clone(), other.clone()], "anthropic"), "one not heard from yet is not taken to have none");
        s.heard_from(&other, &offered(false));
        assert!(s.key_wanted(&[main.clone(), other.clone()], "anthropic"));
        s.heard_from(&gone, &offered(false));
        assert!(!s.key_wanted(&[main.clone()], "anthropic"), "a computer since unpaired counts for nothing");
    }

    #[test]
    fn what_he_chose_while_the_file_could_not_be_read_is_kept_over_what_it_holds() {
        // (audit 8, M4)
        let pick = |id: &str| Pick { place: ChoicePlace::Phone, id: id.into(), offer: String::new(), set: BTreeMap::new(), voice: String::new() };
        let mut found = Saved { ears: Some(pick("kept-ears")), voice: Some(pick("kept-voice")), tools: vec!["codex".into()], ..Saved::default() };
        let before = Saved { ears: Some(pick("usual")), voice: Some(pick("usual")), ..Saved::default() };
        let mut since = Saved { voice: Some(pick("chosen-since")), ..before.clone() };
        let mut touched = Touched::default();
        touched.add(&before, &since);
        since.touched = touched;
        found.rejoin(since);
        assert_eq!(found.ears, Some(pick("kept-ears")), "a usual one put in at open replaced what he had chosen");
        assert_eq!(found.voice, Some(pick("chosen-since")));
        assert_eq!(found.tools, ["codex"]);
    }
}
