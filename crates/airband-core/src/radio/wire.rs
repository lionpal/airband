//! Every request a radio or an agent's reporter makes of a tower, typed once here and used by both ends.
//!
//! Each route is one [`Endpoint`] in [`at`]: its method, its path, and the types of its query, body and answer.
//! The rule on the wire: a 200 carries the endpoint's answer and nothing else; a refusal is a 4xx carrying a
//! [`Refusal`]; a 200 never carries an error. "Nothing to say back" is [`Done`]. A tower and a radio that speak
//! different versions of this ([`super::SCHEMA`]) say so with [`Refusal::OtherVersion`] and go no further.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The methods the routes use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    Post,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Delete => "DELETE",
        }
    }
}

impl std::fmt::Display for Method {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One route of a tower.
pub trait Endpoint {
    const METHOD: Method;
    /// The whole path. `{id}` stands for the one part that names something (see [`path_of`]).
    const PATH: &'static str;
    /// What goes in the address after `?`; [`Nothing`] when nothing does.
    type Query: Serialize + DeserializeOwned + Default;
    /// What is sent; [`Nothing`] when nothing is.
    type Body: Serialize + DeserializeOwned;
    /// What a 200 carries.
    type Answer: Serialize + DeserializeOwned;
}

/// Why a tower would not do what was asked: what a 4xx carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "refusal", rename_all = "snake_case")]
pub enum Refusal {
    /// It could not be done, and why, in words for him.
    Told { reason: String },
    /// The request itself was wrong: a query value that is not a number, a body that is not the endpoint's.
    BadAsk { reason: String },
    /// The radio speaks another version of the protocol: this one.
    OtherVersion { speaks: String },
    /// Not now, for the reason given, in words for him: it may work later, and is not a no.
    NotNow { reason: String },
    /// Too big to pass through the relay.
    TooBig,
    /// What it names is not there (any more).
    Gone,
    /// Only a radio may ask this, or only the tower's own reporters.
    NotAllowed,
    /// A refusal this build does not know.
    #[serde(other)]
    Unknown,
}

impl Refusal {
    pub fn told(reason: impl Into<String>) -> Self {
        Refusal::Told { reason: reason.into() }
    }
    /// The status it is sent with. A reader goes by the body's tag, never by the status.
    pub fn status(&self) -> u16 {
        match self {
            Refusal::Told { .. } => 409,
            Refusal::BadAsk { .. } | Refusal::Unknown => 400,
            Refusal::OtherVersion { .. } => 426,
            Refusal::NotNow { .. } => 503,
            Refusal::TooBig => 413,
            Refusal::Gone => 404,
            Refusal::NotAllowed => 403,
        }
    }
    /// What to tell him, when the tower gave words for it.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Refusal::Told { reason } | Refusal::BadAsk { reason } | Refusal::NotNow { reason } => Some(reason),
            _ => None,
        }
    }
}

/// What a route that takes no query or no body is given: an empty object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nothing {}

/// The answer of a route that has nothing to say back but that it was done.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {}

/// `GET next`: what the radio says as it asks for the next transmission.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextQuery {
    /// He has just turned reading aloud on: what was said long before is not read out now.
    #[serde(default)]
    pub joined: bool,
    /// The id of the transmission this radio last got: one handed over since and never heard of was lost on the
    /// way, and is handed over again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub have: Option<String>,
}

/// `GET feed`: which part of the chat, from where, and how long to wait for something new.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedQuery {
    /// Only this team's, by its id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<crate::ids::TeamId>,
    /// Only this agent's, by its id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<crate::ids::AgentId>,
    /// Everything new or changed after this change count. 0 starts from nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    /// With no `since`: the lines added after this position.
    #[serde(default)]
    pub after: u64,
    /// The most lines to send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// The roster's change count this radio last saw: a change to the roster alone also ends the wait.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roster: Option<u64>,
    /// How many seconds to wait for something new; no more than [`FeedQuery::WAIT_MOST`].
    #[serde(default)]
    pub wait: u32,
}

impl FeedQuery {
    /// The longest a request for the feed is held open, in seconds.
    pub const WAIT_MOST: u32 = 25;
    /// How long the tower holds the request open: what was asked, no more than [`FeedQuery::WAIT_MOST`].
    pub fn wait_for(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.wait.min(Self::WAIT_MOST).into())
    }
}

/// `GET message/{id}`: which version of one message.
#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct MessageQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form: Option<super::Form>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<super::Part>,
}

/// `GET message/{id}`'s answer: that version's words.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageText {
    pub text: String,
}

/// `GET picture/{id}`: the part starting at this byte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartQuery {
    #[serde(default)]
    pub from: usize,
}

/// `POST attachment`'s answer: how much of the file is whole from its start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentNext {
    pub next: u64,
}

/// `POST reads`'s answer: what now reads for this tower, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadsSet {
    pub reads: String,
}

/// `POST ready`'s answer: how many transmissions are still to be heard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyAnswer {
    pub waiting: usize,
}

/// `GET accounts`: read them again now, not from what was kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountsQuery {
    #[serde(default)]
    pub fresh: bool,
}

/// `GET permission/{id}`: how many seconds to wait for his answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitQuery {
    #[serde(default)]
    pub wait: u64,
}

macro_rules! endpoints {
    ($($(#[$doc:meta])* $name:ident: $method:ident $path:literal, $q:ty, $b:ty => $a:ty;)*) => {
        $(
            $(#[$doc])*
            #[derive(Debug, Clone, Copy)]
            pub struct $name;
            impl super::Endpoint for $name {
                const METHOD: super::Method = super::Method::$method;
                const PATH: &'static str = $path;
                type Query = $q;
                type Body = $b;
                type Answer = $a;
            }
        )*
        /// Every route: its name, method and path.
        pub const ALL: &[(&str, super::Method, &str)] = &[$((stringify!($name), super::Method::$method, $path),)*];
    };
}

/// The routes, one type each.
pub mod at {
    use super::*;
    use crate::agent::{AgentEvent, PermissionAsk, PermissionState};
    use crate::radio::*;

    endpoints! {
        /// Held open up to 25 seconds; `null` when nothing is waiting.
        Next: Get "/v1/radio/next", NextQuery, Nothing => Option<Transmission>;
        Heard: Post "/v1/radio/heard", Nothing, crate::radio::Heard => Done;
        Unheard: Post "/v1/radio/unheard", Nothing, crate::radio::Unheard => Done;
        Presence: Post "/v1/radio/presence", Nothing, crate::radio::Presence => Done;
        Ready: Post "/v1/radio/ready", Nothing, Nothing => ReadyAnswer;
        Utterance: Post "/v1/radio/utterance", Nothing, crate::radio::Utterance => Outcome;
        Feed: Get "/v1/radio/feed", FeedQuery, Nothing => crate::radio::Feed;
        Picture: Get "/v1/radio/picture/{id}", PartQuery, Nothing => PicturePart;
        ReadsOffered: Get "/v1/radio/reads", Nothing, Nothing => crate::radio::ReadsOffered;
        Reads: Post "/v1/radio/reads", Nothing, SetReads => ReadsSet;
        Message: Get "/v1/radio/message/{id}", MessageQuery, Nothing => MessageText;
        Attachment: Post "/v1/radio/attachment", Nothing, AttachmentPart => AttachmentNext;
        Accounts: Get "/v1/radio/accounts", AccountsQuery, Nothing => crate::radio::Accounts;
        UseAccount: Post "/v1/radio/accounts/use", Nothing, crate::radio::UseAccount => crate::radio::Accounts;
        Team: Post "/v1/radio/team", Nothing, MoveAgent => Moved;
        RenameTeam: Post "/v1/radio/team/rename", Nothing, crate::radio::RenameTeam => TeamRenamed;
        Folders: Post "/v1/radio/folders", Nothing, FoldersAsk => crate::radio::Folders;
        Recent: Get "/v1/radio/recent", Nothing, Nothing => crate::radio::Recent;
        Project: Post "/v1/radio/project", Nothing, NewProject => ProjectMade;
        TakeOver: Post "/v1/radio/takeover", Nothing, crate::radio::TakeOver => Done;
        Stop: Post "/v1/radio/stop", Nothing, StopAgent => Done;
        Unsend: Post "/v1/radio/unsend", Nothing, crate::radio::Unsend => Done;
        Dismiss: Post "/v1/radio/dismiss", Nothing, crate::radio::Dismiss => Done;
        Taken: Post "/v1/radio/taken", Nothing, crate::radio::Taken => Done;
        End: Post "/v1/radio/end", Nothing, EndAgent => Done;
        Push: Post "/v1/radio/push", Nothing, PushTarget => Done;
        Tools: Post "/v1/radio/tools", Nothing, UsesTools => Done;
        /// An agent's reporter: one event.
        Events: Post "/v1/agent/events", Nothing, AgentEvent => Done;
        /// A held agent's program is gone.
        HeldEnd: Delete "/v1/agent/held/{id}", Nothing, Nothing => Done;
        Permission: Post "/v1/agent/permission", Nothing, PermissionAsk => PermissionState;
        PermissionAnswer: Get "/v1/agent/permission/{id}", WaitQuery, Nothing => PermissionState;
        PermissionWithdraw: Delete "/v1/agent/permission/{id}", Nothing, Nothing => Done;
    }
}

/// An endpoint's path with the thing it names put in place of `{id}`.
pub fn path_of<E: Endpoint>(id: &str) -> String {
    E::PATH.replacen("{id}", &encode(id), 1)
}

/// An endpoint's whole address: its path (with `id` in place of `{id}`) and its query.
pub fn address<E: Endpoint>(id: Option<&str>, query: &E::Query) -> String {
    let path = id.map_or_else(|| E::PATH.to_string(), path_of::<E>);
    let q = query_of(query);
    if q.is_empty() { path } else { format!("{path}?{q}") }
}

/// A query type written as the part of an address after `?`: each field that has a value, in order. A field
/// with none is left out, so the reader takes its default.
pub fn query_of<Q: Serialize>(query: &Q) -> String {
    let serde_json::Value::Object(fields) = serde_json::to_value(query).unwrap_or_default() else { return String::new() };
    fields
        .into_iter()
        .filter_map(|(k, v)| {
            let v = match v {
                serde_json::Value::Null => return None,
                serde_json::Value::String(s) => s,
                serde_json::Value::Bool(false) => return None,
                other => other.to_string(),
            };
            Some(format!("{}={}", encode(&k), encode(&v)))
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Percent-encoding for one part of an address: everything but letters, digits and `-._~`.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests;
