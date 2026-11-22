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
    pub(crate) fn body_for(&self, phone_key: Option<&str>, there: &ReadsOffered) -> Result<airband_core::radio::SetReads, String> {
        let offer = there.offers.iter().find(|r| r.provider == self.id && r.offer == self.offer);
        let needs_key = offer.map_or_else(|| airband_core::providers::provider(&self.id).is_some_and(|p| p.key), |r| r.key);
        // a computer's own key is its own: the phone's goes only where there is none
        let key = match phone_key.filter(|k| !k.is_empty()) {
            _ if !needs_key || offer.is_some_and(|r| r.has_key) => String::new(),
            Some(key) => key.to_string(),
            None => return Err(NO_KEY.into()),
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
            self.reads = crate::services::services(Job::Summary).into_iter().find(|s| !most.is_empty() && s.login == most).map(|s| Intent::after(before.as_ref(), ReadsWanted { id: s.kind, offer: s.offer, set: BTreeMap::new(), his: false }));
        }
    }
}
