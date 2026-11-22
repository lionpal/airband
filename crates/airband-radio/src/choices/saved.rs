//! What he picked, as it is kept in `choices.json`.
use super::*;

/// What he picked for a job done on this device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Pick {
    pub(super) place: ChoicePlace,
    /// The model's kind or the provider's id. Empty for his computer's choice.
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

/// What is to write his summaries, chosen while no computer could be reached. It is sent to each computer as
/// it can be reached.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Waiting {
    /// The provider's id, and which of its offers.
    pub(super) id: String,
    #[serde(default)]
    pub(super) offer: String,
    #[serde(default)]
    pub(super) set: BTreeMap<String, String>,
    /// He picked it himself. Otherwise it follows from the coding tool he said he uses most, and changes when
    /// that does.
    #[serde(default)]
    pub(super) his: bool,
    /// The computers that have taken it, by `Station::id`. It waits for the rest.
    #[serde(default)]
    pub(super) taken: Vec<String>,
    /// The computers that said no to it, by `Station::id`: they are not asked again, and he was told why.
    #[serde(default)]
    pub(super) refused: Vec<String>,
}

impl Waiting {
    /// The same choice, whichever computers have taken it.
    pub(super) fn same(&self, other: &Waiting) -> bool {
        (&self.id, &self.offer, &self.set) == (&other.id, &other.offer, &other.set)
    }
    /// That computer has given its answer to it, yes or no: it is not asked again.
    pub(super) fn answered(&self, computer: &str) -> bool {
        self.taken.iter().chain(&self.refused).any(|c| c == computer)
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
    pub(super) reads: Option<Waiting>,
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
        if self.offered.is_none() && self.reads.as_ref().is_none_or(|w| !w.his) {
            let most = self.tools.first().cloned().unwrap_or_default();
            self.reads = crate::services::services(Job::Summary).into_iter().find(|s| !most.is_empty() && s.login == most).map(|s| Waiting { id: s.kind, offer: s.offer, set: BTreeMap::new(), his: false, taken: Vec::new(), refused: Vec::new() });
        }
    }
}
