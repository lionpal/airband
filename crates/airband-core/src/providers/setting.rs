//! A setting in use: which provider and offer, with what key, and what was set differently from the row.
use super::*;

/// A provider in use for one job: which row and which of its offers, with what key, and whatever is set
/// differently from the offer. Written in the settings as `{"kind": …, "offer": …, "key": …, …}`. The key is
/// the provider's: the same one whichever of its offers is used.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Use {
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub offer: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    #[serde(flatten)]
    pub set: BTreeMap<String, Value>,
}

/// Everything but the key, which is never printed.
impl std::fmt::Debug for Use {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Use").field("kind", &self.kind).field("offer", &self.offer).field("key", &if self.key.is_empty() { "" } else { "…" }).field("set", &self.set).finish()
    }
}

impl Use {
    /// The word for no provider, where he types or reads a choice (`airband reads none`). No provider, offer or
    /// setting may be called it.
    pub const NONE: &'static str = "none";

    /// Is this a name no provider, offer or setting may take: [`Use::NONE`], or the name of one of the fields a
    /// `Use` is written down with, which a setting of that name would stand in the place of. The fields are read
    /// from the type itself, as it is written.
    pub fn reserved(name: &str) -> bool {
        let written = serde_json::to_value(Use { kind: "x".into(), offer: "x".into(), key: "x".into(), set: BTreeMap::new() }).unwrap_or_default();
        name == Use::NONE || written.as_object().is_some_and(|fields| fields.contains_key(name))
    }

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
            Some(set) => crate::catalog::scalar_text(Some(set)),
            None => self.offer(job).and_then(|(_, o)| o.usual(field)).unwrap_or_default(),
        }
    }
    /// One line for him: the provider and what it is set to. Never the key.
    pub fn describe(&self, job: Job) -> String {
        match self.offer(job) {
            None if self.kind.is_empty() => Use::NONE.into(),
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
