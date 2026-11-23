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
    /// Who it is from, the same for every notice of that agent on that computer: the thread it is grouped in,
    /// and the sender the system groups it under.
    pub thread: String,
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
        let room = n.team.as_ref().map(|t| room_id(&StationId::from(keys.channel.as_str()), t.as_ref())).unwrap_or_default();
        let agent = n.agent.unwrap_or_default();
        Some(OpenedNotice {
            thread: format!("{}/{agent}", keys.channel),
            colour: colours.get(&room).copied(),
            room,
            station: keys.channel,
            message: n.id,
            title: n.from.clone(),
            subtitle: if several { n.station } else { String::new() },
            body: n.text,
            badge: n.badge,
            agent,
            question: n.kind == Some(Kind::NeedsYou),
            options: if n.permission { Vec::new() } else { n.options },
            permission: n.permission,
        })
    })
}

/// The id of the button for the answer at `index` among a notification's answers. [`notice_tap`] reads it back.
#[uniffi::export]
pub fn notice_option_id(index: u32) -> String {
    format!("{OPTION}{index}")
}
/// What a notification button stood for, from the id it was made with: allow, deny, or one of the answers it
/// offered. Nothing for any other id (the reply box, or a tap on the notification itself).
#[uniffi::export]
pub fn notice_tap(action: String) -> Option<Tapped> {
    match action.as_str() {
        ALLOW_ID => Some(Tapped::Allow),
        DENY_ID => Some(Tapped::Deny),
        _ => action.strip_prefix(OPTION).and_then(|n| n.parse().ok()).map(|index| Tapped::Option { index }),
    }
}
/// One button under a question: its words, and what it stands for.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AnswerButton {
    pub words: String,
    pub tapped: Tapped,
}
/// The buttons under a question, in order: Allow and Deny for a request for leave, else the answers it offered.
/// The one rule, for the chat and for a notification.
#[uniffi::export]
pub fn answer_buttons(permission: bool, options: Vec<String>) -> Vec<AnswerButton> {
    if permission {
        return [Tapped::Allow, Tapped::Deny].into_iter().map(|tapped| AnswerButton { words: tapped.words(), tapped }).collect();
    }
    options.into_iter().enumerate().map(|(i, words)| AnswerButton { words, tapped: Tapped::Option { index: i as u32 } }).collect()
}
/// The button of a notification that was tapped, from the id it was made with and the answers the question
/// offered: its words and what it stands for. Nothing for any other id, or an answer it did not offer.
#[uniffi::export]
pub fn notice_button(action: String, options: Vec<String>) -> Option<AnswerButton> {
    let tapped = notice_tap(action)?;
    let words = match &tapped {
        Tapped::Allow | Tapped::Deny => tapped.words(),
        Tapped::Option { index } => options.into_iter().nth(*index as usize)?,
    };
    Some(AnswerButton { words, tapped })
}
/// The name of the set of buttons for these answers: the same answers always get the same name.
#[uniffi::export]
pub fn notice_answers_name(options: Vec<String>) -> String {
    format!("{ANSWERS}{}", crate::desk::same_name(0xcbf2_9ce4_8422_2325, options.join("\u{1}").as_bytes()))
}
/// The set of buttons is one made for a question's answers ([`notice_answers_name`]): one of those the host
/// lets go of once there are many.
#[uniffi::export]
pub fn notice_is_answers(name: String) -> bool {
    name.strip_prefix(ANSWERS).is_some_and(|h| !h.is_empty() && u64::from_str_radix(h, 16).is_ok())
}
/// The ids notification buttons are made with.
#[uniffi::export]
pub fn notice_allow_id() -> String {
    ALLOW_ID.into()
}
#[uniffi::export]
pub fn notice_deny_id() -> String {
    DENY_ID.into()
}
const ALLOW_ID: &str = "allow";
const DENY_ID: &str = "deny";
const OPTION: &str = "option.";
const ANSWERS: &str = "airband.o.";

/// An agent's state, as it is said to someone who cannot see its dot.
#[uniffi::export]
pub fn spoken_state(state: AgentState) -> String {
    match state {
        AgentState::Asking => "waiting on you",
        AgentState::Stalled => "stopped",
        AgentState::Working => "working",
        AgentState::Idle => "idle",
        AgentState::Unknown => "state not known",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_button_id_reads_back_as_what_it_was_made_for_and_no_other_id_does() {
        assert_eq!(notice_tap(notice_option_id(2)), Some(Tapped::Option { index: 2 }));
        assert_eq!(notice_tap(notice_allow_id()), Some(Tapped::Allow));
        assert_eq!(notice_tap(notice_deny_id()), Some(Tapped::Deny));
        assert_eq!(notice_tap("reply".into()), None);
        assert_eq!(notice_tap("option.x".into()), None);
        let name = notice_answers_name(vec!["Yes".into(), "No".into()]);
        assert_eq!(name, notice_answers_name(vec!["Yes".into(), "No".into()]));
        assert!(notice_is_answers(name));
        assert!(!notice_is_answers("airband.reply".into()));
    }
}
