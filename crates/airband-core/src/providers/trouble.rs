//! What went wrong when a service did not do what it was asked, the same for a key being tested and a model
//! being asked to read, and the words he is told it in.
use super::*;

/// Why a service did not do what it was asked: the same whether a key of his was being tested or a model was
/// being asked to read. What he is told about each is written here and nowhere else.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "why", rename_all = "snake_case")]
pub enum Trouble {
    /// It could not be reached.
    Unreachable,
    /// It does not know the key, or will not take it (401, 403).
    WrongKey,
    /// It knows the key and will not serve it: out of credit, over a limit, not allowed (402, 429).
    Refused,
    /// It does not know the model, or is not at the address it was asked at (404).
    NotFound,
    /// It answered with a status nothing here accounts for.
    Unexpected { status: u16 },
    /// It answered, with nothing that can be used.
    NoAnswer,
    /// It is a program on his computer, and did not start or stopped with an error.
    Program,
    /// It took longer than it is given.
    TooSlow,
    /// Nothing is chosen, or what is chosen is not something this version can use.
    NotSet,
}

/// A service as far as telling him what went wrong goes: what it is called, and which of its statuses its row
/// says mean a wrong key or a refusal.
#[derive(Clone, Debug, Default)]
pub struct Answering {
    pub name: String,
    pub check: Option<Check>,
}

impl Answering {
    pub fn of(provider: &Provider) -> Answering {
        Answering { name: provider.name.clone(), check: provider.check.clone() }
    }
    /// A request to it, sent: what came back when it answered 200, else what went wrong, in words for him.
    /// The one place a listening or voice service's reach and status are read.
    pub fn sent(&self, res: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<ureq::http::Response<ureq::Body>, String> {
        let res = res.map_err(|_| Trouble::Unreachable.said(&self.name))?;
        match res.status().as_u16() {
            200 => Ok(res),
            code => Err(Trouble::of_status(code, self.check.as_ref()).said(&self.name)),
        }
    }
    /// It answered, with nothing that can be used.
    pub fn no_answer(&self) -> String {
        Trouble::NoAnswer.said(&self.name)
    }
}

impl Trouble {
    /// What a status says went wrong. A service's own row says which of its statuses mean a wrong key and
    /// which a refusal (`check`); for anything the row does not list, and for a service with no such row, the
    /// status is read as HTTP itself defines it.
    pub fn of_status(status: u16, check: Option<&Check>) -> Trouble {
        // the row's reading first, through the one place a row's statuses are read
        match check.map(|c| c.read(status)) {
            Some(KeyCheck::WrongKey) => return Trouble::WrongKey,
            Some(KeyCheck::Refused) => return Trouble::Refused,
            _ => {}
        }
        match status {
            401 | 403 => Trouble::WrongKey,
            402 | 429 => Trouble::Refused,
            404 => Trouble::NotFound,
            _ => Trouble::Unexpected { status },
        }
    }
    /// What happened, for him. `service` is what the service is called where he chose it.
    pub fn said(self, service: &str) -> String {
        let service = if service.is_empty() { "The service" } else { service };
        match self {
            Trouble::Unreachable => format!("{service} could not be reached."),
            Trouble::WrongKey => format!("{service} did not accept the key."),
            Trouble::Refused => format!("{service} knows the key and would not serve it. The account may be out of credit, or not allowed to use this."),
            Trouble::NotFound => format!("{service} does not have that model at that address."),
            Trouble::Unexpected { status } => format!("{service} gave an answer airband did not expect (status {status})."),
            Trouble::NoAnswer => format!("{service} answered with nothing that could be used."),
            Trouble::Program => format!("{service} did not run on your computer."),
            Trouble::TooSlow => format!("{service} took too long to answer."),
            Trouble::NotSet => "Nothing that this version of airband can use is chosen.".to_string(),
        }
    }
    /// The same when it was a key of his being tested: what happened, and what to do about the key.
    pub fn said_of_key(self, service: &str) -> String {
        let then = match self {
            Trouble::WrongKey => "Check it and paste it again.",
            Trouble::Unreachable => "Check your connection and try again.",
            _ => "The key was not kept.",
        };
        format!("{} {then}", self.said(service))
    }
}
