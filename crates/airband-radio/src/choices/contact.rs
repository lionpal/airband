//! Writing to the person who makes airband: from the bottom of every list he picks from, when what he wants is
//! not on it, and from Settings. Who that is and where a text goes are the catalogue's (`catalog/contact/`);
//! the host only opens a new text message with what this gives it.
use super::*;

/// Where he writes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ContactFrom {
    /// The list of coding tools.
    Tools,
    /// The list of what does one of the jobs he chooses for.
    Job { job: ChoiceJob },
    /// Settings, with nothing in particular in mind.
    Settings,
}

/// A text message to the person who makes airband, ready to open.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ContactNote {
    /// What the row that opens it says.
    pub label: String,
    /// The number it goes to.
    pub to: String,
    /// What the message starts with, for him to finish. Empty when there is nothing to say for him.
    pub body: String,
}

/// The text message to open from `from`. Nothing when this build names no one to write to.
pub(crate) fn note(from: ContactFrom) -> Option<ContactNote> {
    let who = airband_core::catalog::contact()?;
    let name = who.name.trim();
    let wanted = match from {
        ContactFrom::Tools => Some("a coding agent"),
        ContactFrom::Job { job: ChoiceJob::Reads } => Some("a model for summaries"),
        ContactFrom::Job { job: ChoiceJob::Ears } => Some("a model for listening"),
        ContactFrom::Job { job: ChoiceJob::Voice } => Some("a voice"),
        ContactFrom::Settings => None,
    };
    Some(match wanted {
        Some(wanted) => ContactNote { label: "Ask for another".into(), to: who.sms.clone(), body: format!("Hi {name}, I'd like airband to have {wanted} that isn't on the list: ") },
        None => ContactNote { label: format!("Text {name}"), to: who.sms.clone(), body: format!("Hi {name}, ") },
    })
}

#[uniffi::export]
impl Desk {
    /// A text message to the person who makes airband, from `from`: for the last row of a list he picks from,
    /// and for Settings. Nothing when this build names no one.
    pub fn contact(&self, from: ContactFrom) -> Option<ContactNote> {
        note(from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_list_he_picks_from_ends_with_a_way_to_ask_for_another_that_names_the_list() {
        let who = airband_core::catalog::contact().expect("the catalogue names someone to write to");
        let desk = Desk::alone("", Arc::new(super::super::tests::Host::default()));
        let mut lists: Vec<ContactFrom> = desk.choice_jobs().into_iter().map(|job| ContactFrom::Job { job }).collect();
        lists.push(ContactFrom::Tools);
        let mut bodies = Vec::new();
        for from in lists {
            let note = desk.contact(from).expect("a way to ask");
            assert_eq!((note.label.as_str(), note.to.as_str()), ("Ask for another", who.sms.as_str()), "{from:?}");
            assert!(note.body.starts_with(&format!("Hi {}", who.name)), "{from:?}");
            bodies.push(note.body);
        }
        // each names its own list
        for (body, names) in bodies.iter().zip(["summaries", "listening", "a voice", "a coding agent"]) {
            assert!(body.contains(names), "{body}");
        }
        let settings = desk.contact(ContactFrom::Settings).unwrap();
        assert_eq!(settings.label, format!("Text {}", who.name));
    }
}
