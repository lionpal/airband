//! A notification, opened on the device: what it says and how a button answers it. No desk is needed.
use super::*;

/// What a notification says once this device has opened it: everything a host needs to show it and to send an
/// answer to it, with the app running or not.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OpenedNotice {
    /// The computer it came from, for [`Desk::answer_now`].
    pub station: String,
    /// The message it is about.
    pub message: String,
    /// Who sent it, as the app shows the name.
    pub title: String,
    /// The computer's name, when he has more than one.
    pub subtitle: String,
    pub body: String,
    /// The agent's two characters, and its team's place in the host's list of team colours: nothing for a team
    /// the phone has not given one yet, which is drawn with no team colour.
    pub badge: String,
    pub colour: Option<u8>,
    /// The agent's team and its id: where the app opens when he taps the notification.
    pub room: String,
    pub agent: String,
    /// It is a question to him (not a report).
    pub question: bool,
    /// Answers to offer as buttons.
    pub options: Vec<String>,
    /// It asks permission: the buttons are Allow and Deny, and words of his own refuse it.
    pub permission: bool,
}
/// What names a computer to this device, from its pairing link: nothing secret, the same every time.
pub(super) fn station_key(link: &str) -> Option<String> {
    match Pairing::parse(link).ok()? {
        Pairing::Relay { secret, .. } | Pairing::Near { secret, .. } => Keys::derive(&secret).ok().map(|k| k.channel),
        Pairing::Direct { .. } => None,
    }
}
/// What a computer is known by in the rooms of his screen: its relay channel, or for a plain direct pairing,
/// a hash of its link (the link holds a token, and a room's id is kept on disk).
pub(super) fn station_id(link: &str) -> String {
    station_key(link).unwrap_or_else(|| airband_core::relay::token_hash(link))
}
/// A room on his screen: one team on one computer, as one id. Only ever compared whole.
pub(crate) fn room_id(station: &str, team: &str) -> String {
    if team.is_empty() { String::new() } else { format!("{station}/{team}") }
}
/// What a button under a question stood for.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Tapped {
    /// Yes to a request for leave.
    Allow,
    /// No to one.
    Deny,
    /// One of the answers the message offered, by its place in the list.
    Option { index: u32 },
}
impl Tapped {
    pub(super) fn act(&self) -> Act {
        match self {
            Tapped::Allow => Act::Verdict { allow: true },
            Tapped::Deny => Act::Verdict { allow: false },
            Tapped::Option { index } => Act::Option { index: *index },
        }
    }
    /// What the chat shows for a yes or a no. Nothing for an option, whose words are the message's.
    pub(super) fn words(&self) -> String {
        match self {
            Tapped::Allow => airband_core::radio::ALLOW.to_string(),
            Tapped::Deny => airband_core::radio::DENY.to_string(),
            Tapped::Option { .. } => String::new(),
        }
    }
}
/// The word on the button that allows a request for leave, or on the one that refuses it.
#[uniffi::export]
pub fn leave_word(allow: bool) -> String {
    if allow { airband_core::radio::ALLOW } else { airband_core::radio::DENY }.to_string()
}
/// Open a sealed notification with whichever pairing sealed it. `links` are the pairing links the host keeps;
/// `sealed` is the notification as the push service delivered it. Nothing when none of them opens it. This
/// needs no desk: a host calls it from wherever its system hands it a notification. `colours` is each team's
/// colour as [`DeskView::colours`] last gave it, from the copy the host keeps; a team not in it has none.
#[uniffi::export]
pub fn open_notice(links: Vec<String>, sealed: String, colours: std::collections::HashMap<String, u8>) -> Option<OpenedNotice> {
    let sealed = airband_core::encode::unbase64(&sealed)?;
    let several = links.len() > 1;
    links.iter().find_map(|link| {
        let Pairing::Relay { secret, .. } = Pairing::parse(link).ok()? else { return None };
        let keys = Keys::derive(&secret).ok()?;
        let n: airband_core::radio::Notice = serde_json::from_slice(&keys.open(Direction::Push, &sealed).ok()?).ok()?;
        let room = room_id(&keys.channel, &n.team);
        Some(OpenedNotice {
            colour: colours.get(&room).copied(),
            room,
            station: keys.channel,
            message: n.id,
            title: n.from.clone(),
            subtitle: if several { n.station } else { String::new() },
            body: n.text,
            badge: n.badge,
            agent: n.agent.unwrap_or_default(),
            question: n.kind == Some(Kind::NeedsYou),
            options: if n.permission { Vec::new() } else { n.options },
            permission: n.permission,
        })
    })
}

/// What a team he names this way would be called: the computer names it by the same rule. Empty when the
/// words make no name.
#[uniffi::export]
pub fn team_name(written: String) -> String {
    airband_core::routing::team_name(&written).unwrap_or_default()
}

/// Is this a pairing link a computer gave (`airband pair`)? Read by the one parser that pairs with it.
#[uniffi::export]
pub fn is_pairing_link(link: String) -> bool {
    airband_core::pairing::Pairing::parse(link.trim()).is_ok()
}

