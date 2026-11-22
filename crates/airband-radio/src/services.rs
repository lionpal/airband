//! The services on offer for each job, as the provider table lists them and this build can use them; the
//! coding tools airband works with; and a setting made from what he picked. What where he chooses shows.
use std::collections::HashMap;

/// A service for one job, as a provider's row offers it: what where he chooses (`choices.rs`) lists.
#[derive(Debug, Clone)]
pub(crate) struct Service {
    /// Its provider's id. It also names the key: one key serves everything its provider does.
    pub kind: String,
    /// Which of that provider's offers it is.
    pub offer: String,
    /// What to show him: the maker and the model.
    pub name: String,
    /// Not one of the few shown first: offered behind "More services".
    pub more: bool,
    /// It costs him nothing to use: say so.
    pub free: bool,
    /// The coding tool whose sign-in on his computer it uses, by that tool's id; empty when it uses none.
    /// Offer it only to someone who uses that tool.
    pub login: String,
    /// What it costs him, in a few words. Empty when free or not known.
    pub price: String,
    /// For a voice service: a few voices to choose from, the usual one first. The `name` of the one chosen
    /// is the setting `voice`; `label` is what to call it.
    pub voices: Vec<crate::models::VoiceChoice>,
    /// One line under that, for one that takes no key: where it runs and what it costs him. Else empty.
    pub about: String,
    /// It takes a key.
    pub key: bool,
    /// What its key is called, for asking for it ("API key").
    pub key_name: String,
    /// Where its keys are made.
    pub keys_at: String,
    /// What else it can be told. Anything left out is the usual.
    pub asks: Vec<ServiceAsk>,
    /// For a voice service: the voice used until he picks one, as its row gives it.
    pub usual_voice: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ServiceAsk {
    pub field: String,
    pub question: String,
    pub usual: String,
}

impl From<&airband_core::providers::Ask> for ServiceAsk {
    fn from(a: &airband_core::providers::Ask) -> Self {
        ServiceAsk { field: a.field.clone(), question: a.question.clone(), usual: a.usual.clone() }
    }
}

/// The services for a job that this build has the code for, from the provider table, in the order to show.
pub(crate) fn services(job: airband_core::providers::Job) -> Vec<Service> {
    airband_core::providers::in_order(job)
        .into_iter()
        // what this build has the code for. Reading is his computer's job: every one is listed, and the
        // computer says no to one it cannot use.
        .filter(|(_, o)| crate::can_use(o))
        .map(|(p, o)| Service {
            kind: p.id.clone(),
            offer: o.id.clone(),
            more: !o.preferred,
            free: o.local,
            login: o.login.clone(),
            price: o.price.clone(),
            voices: o.voices.iter().map(|v| crate::models::VoiceChoice { name: v.id.clone(), label: v.name.clone(), about: v.about.clone(), preview: v.sample.clone() }).collect(),
            name: o.label.clone(),
            about: o.about.clone(),
            key: p.key,
            key_name: p.key_name.clone(),
            keys_at: p.keys_at.clone(),
            asks: o.asks.iter().map(ServiceAsk::from).collect(),
            usual_voice: o.usual(crate::models::field::VOICE).unwrap_or_default(),
        })
        .collect()
}

/// A coding tool airband works with, for where he picks the ones he uses.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CodingTool {
    /// What its agents are known by (`DeskMember::harness`), and what a service's `login` names.
    pub id: String,
    pub name: String,
    pub maker: String,
    /// What he has to do once before its agents are heard; empty when there is nothing.
    pub first: String,
}

/// The coding tools airband works with, in the order to show them.
#[uniffi::export]
pub fn coding_tools() -> Vec<CodingTool> {
    airband_core::catalog::current().tools.iter().map(|t| CodingTool { id: t.id.clone(), name: t.name.clone(), maker: t.maker.clone(), first: t.first.clone() }).collect()
}

/// What a coding tool is called, by its id, as the catalogue names it. Nothing for a tool the catalogue on this
/// phone does not have.
pub(crate) fn tool_name(id: &str) -> Option<String> {
    airband_core::catalog::current().tools.iter().find(|t| t.id == id).map(|t| t.name.clone())
}

pub(crate) fn service_of(kind: &str, offer: &str, key: &str, settings: HashMap<String, String>) -> airband_core::providers::Use {
    airband_core::providers::Use { kind: kind.into(), offer: offer.into(), key: key.trim().into(), set: settings.into_iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k, serde_json::Value::String(v.trim().to_string()))).collect() }
}
