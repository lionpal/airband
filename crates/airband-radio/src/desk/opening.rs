//! Opened from a notification: the app follows that computer's chat at least as far as the notification, then
//! shows the message it was about, or says why it cannot.
use super::*;

/// The message he opened the app from a notification for, until he looks somewhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Opening {
    station: StationId,
    message: String,
    /// The message's change count when the notification was sent.
    rev: u64,
}

/// Where opening a message from a notification stands.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DeskOpening {
    /// The message, by id: the feed has it under this id once it is [`OpeningState::Here`].
    pub message: String,
    pub state: OpeningState,
}

/// Where opening a message from a notification stands.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum OpeningState {
    /// This phone's copy of that computer's chat is older than the notification: it is being followed up to it.
    CatchingUp,
    /// It is in the feed: scroll to it and mark it.
    Here,
    /// It is not in the chat, and why, in words for him.
    Gone { why: OpeningGone, said: String },
}

/// Why a message opened from a notification is not in the chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum OpeningGone {
    /// The computer it came from is no longer paired with this phone.
    NotPaired,
    /// The chat has been followed past the notification and does not hold it: the computer has let it go.
    NotKept,
}

impl OpeningGone {
    fn said(self) -> String {
        match self {
            OpeningGone::NotPaired => UNPAIRED.into(),
            OpeningGone::NotKept => "That message is no longer in the chat.".into(),
        }
    }
}

/// Where opening stands, from what the phone holds: the computers paired, how far each one's chat has been
/// followed, and the messages it holds.
pub(super) fn opening_state(state: &State, all: &[ChatMessage], o: &Opening) -> DeskOpening {
    let gone = |why: OpeningGone| OpeningState::Gone { why, said: why.said() };
    let state = match state.computer(&o.station) {
        None => gone(OpeningGone::NotPaired),
        Some(c) if c.radio.followed_to() < o.rev => OpeningState::CatchingUp,
        Some(_) if all.iter().any(|m| m.id == o.message && (m.station.is_empty() || m.station == o.station.as_str())) => OpeningState::Here,
        Some(_) => gone(OpeningGone::NotKept),
    };
    DeskOpening { message: o.message.clone(), state }
}

#[uniffi::export]
impl Desk {
    /// He opened the app from the notification for `message` (as [`OpenedNotice`] gave it: its computer, its
    /// change count, and its team and agent). What he looks at becomes that agent, and [`DeskView::opening`]
    /// says where the message stands until he looks somewhere else ([`Desk::look`]) or lets it go
    /// ([`Desk::close_opening`]).
    pub fn open_message(&self, station: String, message: String, rev: u64, room: String, agent: String) {
        {
            let mut state = self.shared.state.lock().unwrap();
            let made = state.focus.made.clone();
            state.focus = Focus { room: Some(room).filter(|r| !r.is_empty()), agent: Some(agent).filter(|a| !a.is_empty()), made };
            state.opening = Some(Opening { station: StationId::new(station), message, rev });
        }
        self.shared.host.changed();
    }
    /// Done with the message opened from a notification: it is no longer marked.
    pub fn close_opening(&self) {
        self.shared.state.lock().unwrap().opening = None;
        self.shared.host.changed();
    }
}
