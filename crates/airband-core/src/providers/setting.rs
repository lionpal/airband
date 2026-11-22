//! A setting in use: which provider and offer, with what key, and what was set differently from the row.
use super::*;

/// A provider in use for one job: which row and which of its offers, with what key, and whatever is set
/// differently from the offer. Written in the settings as `{"kind": …, "offer": …, "key": …, …}`. The key is
/// the provider's: the same one whichever of its offers is used.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Use {
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub offer: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    #[serde(flatten)]
    pub set: BTreeMap<String, Value>,
}

impl Use {
    /// This offer of this provider, with no key and everything as usual.
    pub fn of(kind: &str, offer: &str) -> Use {
        Use { kind: kind.into(), offer: offer.into(), key: String::new(), set: BTreeMap::new() }
    }
    /// It names no provider the catalogue has: nothing is set, or what was set is a row this build lacks.
    pub fn is_none(&self) -> bool {
        provider(&self.kind).is_none()
    }
    /// The row for this job, and how the job is done. Nothing when the settings name a provider or an offer
    /// this build does not have, or one that does not do this job.
    pub fn offer(&self, job: Job) -> Option<(Arc<Provider>, Arc<Offer>)> {
        let p = provider(&self.kind)?;
        let o = p.offers.iter().find(|o| o.job == job && o.id == self.offer).cloned()?;
        Some((p, o))
    }
    /// A setting for this job: what was set, else what the row usually uses.
    pub fn get(&self, job: Job, field: &str) -> String {
        match self.set.get(field) {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => self.offer(job).and_then(|(_, o)| o.usual(field)).unwrap_or_default(),
        }
    }
    /// One line for him: the provider and what it is set to. Never the key.
    pub fn describe(&self, job: Job) -> String {
        match self.offer(job) {
            None if self.kind.is_empty() => "none".into(),
            None => format!("\"{} {}\", which this version of airband does not have", self.kind, self.offer),
            Some((_, o)) => {
                let told: Vec<String> = o.asks.iter().map(|a| format!("{} {}", a.question.to_lowercase(), self.get(job, &a.field))).collect();
                if told.is_empty() { o.label.clone() } else { format!("{} ({})", o.label, told.join(", ")) }
            }
        }
    }

    /// Every setting of a job as it stands, set or usual: what a phone is told beside its pass.
    pub fn settings(&self, job: Job) -> BTreeMap<String, String> {
        self.offer(job).map(|(_, o)| o.asks.iter().map(|a| (a.field.to_string(), self.get(job, &a.field))).collect()).unwrap_or_default()
    }
}
