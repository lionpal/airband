//! The pass a live listening service gives: asked for with his key, kept for the minutes it lasts.
use super::*;

/// What the phone needs to listen live with a service: which dialect to speak, a pass from the service that
/// expires in minutes (made here, with his key), and whatever that service has to be told (a region, a
/// language, a model).
#[derive(Clone, PartialEq)]
pub(crate) struct Pass {
    pub dialect: String,
    pub token: String,
    pub set: BTreeMap<String, String>,
    /// Names he is likely to say (callsigns, projects), for a recogniser that can be told what to expect.
    pub phrases: Vec<String>,
    /// What the service's row tells its dialect: where its stream is, among the rest.
    pub by: By,
    /// When it ends (ms), as it says itself or as it was asked for; nothing when it does not say.
    pub until: Option<u64>,
}

/// What a dialect is given by its service: the pass, and when it ends when that is known.
pub(crate) struct Ticket {
    pub token: String,
    pub until: Option<u64>,
}

/// When a signed web token says it ends (its `exp`, in ms): nothing for a token that is not one or does not
/// say.
pub(super) fn signed_until(token: &str) -> Option<u64> {
    #[derive(serde::Deserialize)]
    struct Claims {
        exp: u64,
    }
    let mut parts = token.split('.');
    let (_, claims, _, None) = (parts.next()?, parts.next()?, parts.next()?, parts.next()) else { return None };
    let claims: Claims = serde_json::from_slice(&airband_core::encode::unbase64_url(claims)?).ok()?;
    claims.exp.checked_mul(1000)
}

/// The pass itself is never printed.
impl std::fmt::Debug for Pass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pass").field("dialect", &self.dialect).field("token", &"…").field("set", &self.set).field("phrases", &self.phrases).field("until", &self.until).finish_non_exhaustive()
    }
}

impl Pass {
    /// One of the settings, or empty.
    pub(crate) fn get(&self, field: &str) -> &str {
        self.set.get(field).map(String::as_str).unwrap_or_default()
    }

    /// Where the live connection is made, as the row says (`stream`, with his settings filled in where it
    /// names them), and the host in it.
    pub(crate) fn stream(&self) -> Result<(String, String), ()> {
        let url = airband_core::providers::fill(self.by.text(crate::models::field::STREAM), &|field| self.get(field).to_string()).map_err(|_| ())?;
        let host = Some(airband_core::links::site(&url)).filter(|h| !h.is_empty()).ok_or(())?;
        Ok((url, host))
    }
}

/// What he is told when a service answered and gave nothing to listen with.
pub(super) fn no_pass(service: &Provider) -> String {
    airband_core::providers::Answering::of(service).no_answer()
}

/// A pass that a service gives as one field of a JSON answer.
pub(super) fn pass_field(by: &By, service: &Provider, set: &BTreeMap<String, String>, key: &str, body: Option<Value>, field: &str) -> Result<String, String> {
    let mut res = ask_pass(by, service, set, key, body)?;
    res.body_mut().read_json::<Value>().ok().and_then(|v| v[field].as_str().map(String::from)).ok_or_else(|| no_pass(service))
}

/// A pass that a service gives as the whole of its answer.
pub(super) fn pass_whole(by: &By, service: &Provider, set: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    ask_pass(by, service, set, key, None)?.body_mut().read_to_string().map_err(|_| no_pass(service))
}

/// Ask a live service for a pass, the way every dialect here does: one request to where its row says
/// (`pass`), with the key presented as its row says (`auth`), and `body` when the dialect sends one. The
/// answer comes back for the dialect to take its pass from. Err says why not, in words for him, naming the
/// service as its row does.
pub(super) fn ask_pass(by: &By, service: &Provider, set: &BTreeMap<String, String>, key: &str, body: Option<Value>) -> Result<crate::net::Response, String> {
    let url = airband_core::providers::fill(by.text(crate::models::field::PASS), &|field| set.get(field).cloned().unwrap_or_default())?;
    let req = crate::services::keyed(crate::net::Ask::post(&url), by, key)?;
    let req = match body {
        Some(body) => req.json(&body),
        None => req.header("Content-Length", "0"),
    };
    airband_io::http::sent(&airband_core::providers::Answering::of(service), crate::net::Client::new(Duration::from_secs(8)).send(req))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_signed_pass_says_when_it_ends_and_anything_else_says_nothing() {
        // (audit 9, B-32)
        let claims = airband_core::encode::base64_url(br#"{"exp":1900000000,"region":"r"}"#);
        assert_eq!(super::signed_until(&format!("h.{claims}.s")), Some(1_900_000_000_000));
        assert_eq!(super::signed_until("opaque-token"), None);
        assert_eq!(super::signed_until(&format!("h.{}.s", airband_core::encode::base64_url(b"{}"))), None);
        assert_eq!(super::signed_until(&format!("h.{claims}.s.extra")), None);
    }
}
