//! Asking every computer the same thing at once: each is asked whatever the others answer, and what each did
//! is kept by its id.
use super::*;
use std::collections::BTreeMap;

/// What one computer did with what it was asked.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Reach<T> {
    Done(T),
    /// It answered no, with why in words for him: it is not asked again until what is wanted changes.
    Refused(String),
    /// It could not be reached, or had trouble that may pass: it is asked again when it is reached.
    Unreachable,
    /// It was not asked, because it could not take it as things are: why, in words for him.
    Skipped(String),
}

impl<T> Reach<T> {
    /// What a request's answer was, by the kind of error, never its words.
    pub(crate) fn of(answer: Result<T, RadioError>) -> Reach<T> {
        match answer {
            Ok(value) => Reach::Done(value),
            Err(RadioError::Told { reason } | RadioError::BadLink { reason }) => Reach::Refused(reason),
            Err(no @ RadioError::Refused) => Reach::Refused(no.to_string()),
            Err(RadioError::Offline | RadioError::Kept { .. }) => Reach::Unreachable,
        }
    }
    /// Why it was not taken, in words for him, when it said no or could not be asked.
    pub(crate) fn why_not(&self) -> Option<&str> {
        match self {
            Reach::Refused(why) | Reach::Skipped(why) => Some(why),
            Reach::Done(_) | Reach::Unreachable => None,
        }
    }
}

/// Ask each computer, by id, with `op`, all at once: every one is asked, whatever the others answer. Blocks
/// until each has answered or failed.
pub(crate) fn fan_out<T: Send>(targets: Vec<(String, Arc<Radio>)>, op: impl Fn(&Radio) -> Reach<T> + Sync) -> BTreeMap<String, Reach<T>> {
    let op = &op;
    thread::scope(|scope| {
        let asked: Vec<_> = targets.into_iter().map(|(id, radio)| (id, scope.spawn(move || op(&radio)))).collect();
        asked.into_iter().map(|(id, asking)| (id, asking.join().unwrap_or(Reach::Unreachable))).collect()
    })
}
