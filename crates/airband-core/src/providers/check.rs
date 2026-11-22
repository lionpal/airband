//! How a key is checked when he gives it: the request its row names, what came of it, and the settings filled
//! into the address it is sent to.
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

/// What testing a key came to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum KeyCheck {
    /// A setting the address is built from is not set, or is not something it can be: nothing was sent. `why`
    /// says which, in words for him ([`fill`]).
    BadSetting { why: String },
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
    pub fn trouble(&self) -> Option<Trouble> {
        match *self {
            KeyCheck::Good { .. } => None,
            KeyCheck::BadSetting { .. } => Some(Trouble::NotSet),
            KeyCheck::WrongKey => Some(Trouble::WrongKey),
            KeyCheck::Refused => Some(Trouble::Refused),
            KeyCheck::Unreachable => Some(Trouble::Unreachable),
            KeyCheck::Unexpected { status } => Some(Trouble::Unexpected { status }),
        }
    }
    /// What he is told when the key did not pass, in the words every failure of a service is told in. Nothing
    /// for one that did.
    pub fn said(&self, service: &str) -> Option<String> {
        match self {
            KeyCheck::BadSetting { why } => Some(format!("{why} The key was not kept.")),
            other => other.trouble().map(|t| t.said_of_key(service)),
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
    /// The address to ask, with the settings filled in ([`fill`]). An error saying which setting, when it names
    /// one that is not set or one whose value cannot go in an address.
    pub fn address(&self, settings: &BTreeMap<String, String>) -> Result<String, KeyCheck> {
        fill(&self.url, &|field| settings.get(field).cloned().unwrap_or_default()).map_err(|why| KeyCheck::BadSetting { why })
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
