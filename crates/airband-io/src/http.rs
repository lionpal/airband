//! The HTTP client everything uses. A name is found the way the system finds it, and no other way: nothing is
//! asked of a resolver he did not choose.
use airband_core::providers::Answering;
use std::time::Duration;

pub use ureq::Error;
pub type Response = ureq::http::Response<ureq::Body>;

/// The way out to the network: what gives up when, and every HTTP status taken as an answer and not an error.
/// Requests go out only through [`Client::send`], so each is made one way, here.
#[derive(Debug, Clone)]
pub struct Client {
    agent: ureq::Agent,
}

impl Client {
    /// One that gives up after `timeout`.
    pub fn new(timeout: Duration) -> Client {
        Client { agent: ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false).build().into() }
    }
    /// For an address that may not be there at all (his computer's, on a network he may have left): it gives up
    /// on making the connection after `connect`, so finding that out costs a moment and not a minute.
    pub fn quick(timeout: Duration, connect: Duration) -> Client {
        Client { agent: ureq::Agent::config_builder().timeout_global(Some(timeout)).timeout_connect(Some(connect)).http_status_as_error(false).build().into() }
    }
    /// For a download that may take as long as it takes: it gives up only on making the connection after
    /// `connect`, or on hearing nothing back for `first` once it is made.
    pub fn download(connect: Duration, first: Duration) -> Client {
        Client { agent: ureq::Agent::config_builder().timeout_connect(Some(connect)).timeout_recv_response(Some(first)).http_status_as_error(false).build().into() }
    }
    /// Send `ask`. An error is only a place that could not be reached, or a body that could not be written.
    pub fn send(&self, ask: Ask) -> Result<Response, Error> {
        let Ask { verb, url, headers, body } = ask;
        let body = body?;
        macro_rules! with_body {
            ($req:expr) => {{
                let req = headers.iter().fold($req, |r, (k, v)| r.header(k.as_str(), v.as_str()));
                match body {
                    Some(bytes) => req.send(&bytes[..]),
                    None => req.send_empty(),
                }
            }};
        }
        macro_rules! without_body {
            ($req:expr) => {
                headers.iter().fold($req, |r, (k, v)| r.header(k.as_str(), v.as_str())).call()
            };
        }
        match verb {
            Verb::Get => without_body!(self.agent.get(&url)),
            Verb::Delete => without_body!(self.agent.delete(&url)),
            Verb::Post => with_body!(self.agent.post(&url)),
            Verb::Put => with_body!(self.agent.put(&url)),
            Verb::Patch => with_body!(self.agent.patch(&url)),
        }
    }
}

/// What a request does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl From<airband_core::radio::wire::Method> for Verb {
    fn from(m: airband_core::radio::wire::Method) -> Verb {
        use airband_core::radio::wire::Method;
        match m {
            Method::Get => Verb::Get,
            Method::Post => Verb::Post,
            Method::Delete => Verb::Delete,
        }
    }
}

/// One request, made before it is sent: where, how, with which headers and what body.
#[derive(Debug)]
pub struct Ask {
    verb: Verb,
    url: String,
    headers: Vec<(String, String)>,
    /// The bytes to send, none for an empty body; an error when a typed body could not be written.
    body: Result<Option<Vec<u8>>, Error>,
}

impl Ask {
    pub fn to(verb: Verb, url: impl Into<String>) -> Ask {
        Ask { verb, url: url.into(), headers: Vec::new(), body: Ok(None) }
    }
    pub fn get(url: impl Into<String>) -> Ask {
        Ask::to(Verb::Get, url)
    }
    pub fn post(url: impl Into<String>) -> Ask {
        Ask::to(Verb::Post, url)
    }
    pub fn put(url: impl Into<String>) -> Ask {
        Ask::to(Verb::Put, url)
    }
    pub fn patch(url: impl Into<String>) -> Ask {
        Ask::to(Verb::Patch, url)
    }
    pub fn delete(url: impl Into<String>) -> Ask {
        Ask::to(Verb::Delete, url)
    }
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Ask {
        self.headers.push((name.into(), value.into()));
        self
    }
    /// Each of `headers`.
    pub fn headers<K: AsRef<str>, V: AsRef<str>>(mut self, headers: impl IntoIterator<Item = (K, V)>) -> Ask {
        self.headers.extend(headers.into_iter().map(|(k, v)| (k.as_ref().to_string(), v.as_ref().to_string())));
        self
    }
    /// `body` as JSON.
    pub fn json<T: serde::Serialize + ?Sized>(self, body: &T) -> Ask {
        let bytes = serde_json::to_vec(body).map(Some).map_err(Error::Json);
        Ask { body: bytes, ..self }.header("content-type", "application/json; charset=utf-8")
    }
    /// These bytes, as they are, of `kind` (a content type).
    pub fn bytes(self, kind: &str, bytes: Vec<u8>) -> Ask {
        Ask { body: Ok(Some(bytes)), ..self }.header("content-type", kind)
    }
}

/// A request to a listening or voice service, sent: what came back when it answered 200, else what went wrong,
/// in words for him, as the service's row reads its statuses ([`Answering::status`]).
pub fn sent(service: &Answering, res: Result<Response, Error>) -> Result<Response, String> {
    let res = res.map_err(|_| service.unreachable())?;
    service.status(res.status().as_u16()).map(|_| res)
}

/// What one of airband's own typed routes said back ([`airband_core::radio::wire::Endpoint`]).
#[derive(Debug)]
pub enum Said<A> {
    /// 200, with the route's own answer.
    Took(A),
    /// Refused, saying why.
    No(airband_core::radio::wire::Refusal),
    /// Any other status, or an answer that did not read as the route's: the status it came with.
    Trouble(u16),
}

/// Call one of airband's own typed routes at `base` (`http://host:port`), with `headers` on the request: the
/// address, method, query, body and answer all come from the endpoint, so no caller writes a path. An error is
/// only a place that could not be reached.
pub fn call<E: airband_core::radio::wire::Endpoint>(client: &Client, base: &str, headers: &[(&str, &str)], id: Option<&str>, query: &E::Query, body: Option<&E::Body>) -> Result<Said<E::Answer>, Error> {
    use airband_core::radio::wire::{Method, Refusal, address};
    let url = format!("{}{}", base.trim_end_matches('/'), address::<E>(id, query));
    let ask = Ask::to(E::METHOD.into(), url).headers(headers.iter().copied());
    let ask = match (E::METHOD, body) {
        (Method::Post, Some(body)) => ask.json(body),
        _ => ask,
    };
    let mut res = client.send(ask)?;
    let status = res.status().as_u16();
    Ok(match status {
        200 => res.body_mut().read_json::<E::Answer>().map_or(Said::Trouble(status), Said::Took),
        _ => res.body_mut().read_json::<Refusal>().map_or(Said::Trouble(status), Said::No),
    })
}
