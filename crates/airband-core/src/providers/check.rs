//! Whether a service works: how a key is checked when he gives it, what went wrong when one fails, and the
//! settings filled into an address.
use super::*;

/// The ways a key check is asked.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    /// Sent with an empty body.
    Post,
}

/// How a key is tested when he gives it, before it is kept: one request to the service that costs nothing
/// and that a wrong key fails. Part of the provider's row, beside what its key is called.
///
/// `url` may name settings of the offer he is choosing in braces (`{url}/models`, `https://{region}.…`): they
/// are filled with what he set, or the row's usual. What the answer means is said by its status alone, in the
/// lists here. Nothing the service writes in its answer is read.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Check {
    /// How it is asked. A row naming a method this build does not know does not read.
    pub method: Method,
    pub url: String,
    /// How the key is presented for this request.
    pub auth: Auth,
    /// Anything else the service must be sent with it (a version header).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// The statuses that mean the key is good.
    pub good: Vec<u16>,
    /// Those that mean it is not a key the service knows.
    pub wrong_key: Vec<u16>,
    /// Those that mean the service knows the key and will not serve it: no credit, not allowed, too many
    /// requests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refused: Vec<u16>,
    /// This request is one the service charges for: it has none that is free and needs the key. He is told
    /// one was made.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paid: bool,
}

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

/// What testing a key came to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyCheck {
    /// The service took it. `paid`: the request was one it charges for.
    Good { paid: bool },
    /// The service does not know this key.
    WrongKey,
    /// The service knows the key and will not serve it.
    Refused,
    /// The service could not be reached, so nothing is known about the key.
    Unreachable,
    /// The service answered with a status the row does not account for.
    Unexpected { status: u16 },
}

impl KeyCheck {
    /// What went wrong, when the key did not pass.
    pub fn trouble(self) -> Option<Trouble> {
        match self {
            KeyCheck::Good { .. } => None,
            KeyCheck::WrongKey => Some(Trouble::WrongKey),
            KeyCheck::Refused => Some(Trouble::Refused),
            KeyCheck::Unreachable => Some(Trouble::Unreachable),
            KeyCheck::Unexpected { status } => Some(Trouble::Unexpected { status }),
        }
    }
}

/// An address from a row, with the settings it names in braces filled in (`https://{region}.example/v1`,
/// `…/models/{model}:generate`, `{url}/models`). This is the one place a setting becomes part of an address
/// a key is sent to, so what may go in is narrow:
///
/// - A setting inside an address is letters, digits, `.`, `-`, `_` or `:`: nothing that could end a host
///   name or a path and lead a key somewhere else.
/// - A setting the address starts with is the whole address of a service (`{url}/models`): an `http://` or
///   `https://` address with a host and a path and nothing else, no login, question mark or fragment.
///
/// Err says which setting is not that, in words for him.
pub fn fill(template: &str, setting: &dyn Fn(&str) -> String) -> Result<String, String> {
    let part = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':');
    let mut out = String::new();
    let mut rest = template;
    while let Some((before, after)) = rest.split_once('{') {
        let (field, after) = after.split_once('}').ok_or("An address in the catalogue is not well formed.")?;
        let value = setting(field).trim().to_string();
        if out.is_empty() && before.is_empty() {
            let value = value.trim_end_matches('/');
            // a whole web address with no login, question or fragment, made only of characters an address part may have
            let whole = crate::links::web(value).filter(|u| u.username().is_empty() && u.password().is_none() && u.query().is_none() && u.fragment().is_none());
            let plain = value.bytes().all(|b| part(b) || matches!(b, b'/' | b'[' | b']'));
            if whole.is_none() || !plain {
                return Err(format!("\"{value}\" is not something the setting {field} can be. It is the address of a service, like the usual one."));
            }
            out.push_str(value);
        } else {
            if value.is_empty() || !value.bytes().all(part) {
                return Err(format!("\"{value}\" is not something the setting {field} can be. It is letters and digits, like the usual one."));
            }
            out.push_str(before);
            out.push_str(&value);
        }
        rest = after;
    }
    out.push_str(rest);
    Ok(out)
}

impl Check {
    /// The address to ask, with the settings filled in ([`fill`]). Nothing when it names one that is not set,
    /// or one whose value cannot go in an address.
    pub fn address(&self, settings: &BTreeMap<String, String>) -> Option<String> {
        fill(&self.url, &|field| settings.get(field).cloned().unwrap_or_default()).ok()
    }

    /// What a status means.
    pub fn read(&self, status: u16) -> KeyCheck {
        if self.good.contains(&status) {
            KeyCheck::Good { paid: self.paid }
        } else if self.wrong_key.contains(&status) {
            KeyCheck::WrongKey
        } else if self.refused.contains(&status) {
            KeyCheck::Refused
        } else {
            KeyCheck::Unexpected { status }
        }
    }
}
