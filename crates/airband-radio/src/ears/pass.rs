//! The pass a live listening service gives: asked for with his key, kept for the minutes it lasts.
use super::*;

/// What the phone needs to listen live with a service: which dialect to speak, a pass from the service that
/// expires in minutes (made here, with his key), and whatever that service has to be told (a region, a
/// language, a model).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pass {
    pub dialect: String,
    pub token: String,
    pub set: BTreeMap<String, String>,
    /// Names he is likely to say (callsigns, projects), for a recogniser that can be told what to expect.
    pub phrases: Vec<String>,
    /// What the service's row tells its dialect: where its stream is, among the rest.
    pub by: By,
}

impl Pass {
    /// One of the settings, or empty.
    pub(crate) fn get(&self, field: &str) -> &str {
        self.set.get(field).map(String::as_str).unwrap_or_default()
    }

    /// Where the live connection is made, as the row says (`stream`, with his settings filled in where it
    /// names them), and the host in it.
    pub(crate) fn stream(&self) -> Result<(String, String), ()> {
        let url = airband_core::providers::fill(self.by.text("stream"), &|field| self.get(field).to_string()).map_err(|_| ())?;
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
pub(super) fn ask_pass(by: &By, service: &Provider, set: &BTreeMap<String, String>, key: &str, body: Option<Value>) -> Result<ureq::http::Response<ureq::Body>, String> {
    let url = airband_core::providers::fill(by.text("pass"), &|field| set.get(field).cloned().unwrap_or_default())?;
    let auth = by.auth().ok_or("This version of airband was not told how that listening service takes its key.")?;
    let req = crate::net::agent(Duration::from_secs(8)).post(&url).header(auth.header.as_str(), auth.value(key.trim()).as_str());
    airband_core::providers::Answering::of(service).sent(match body {
        Some(body) => req.send_json(&body),
        None => req.header("Content-Length", "0").send_empty(),
    })
}
