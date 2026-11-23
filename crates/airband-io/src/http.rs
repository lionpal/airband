//! The HTTP client everything uses. A name is found the way the system finds it, and no other way: nothing is
//! asked of a resolver he did not choose.
//!
//! Everything a caller holds is this module's own: [`Client`], [`Ask`], [`Response`], [`Body`] and [`Error`].
//! The library underneath is named nowhere else, so it can be changed here alone.
#![expect(clippy::disallowed_methods, clippy::disallowed_types, reason = "the one place the HTTP library is called")]
use airband_core::providers::Answering;
use std::time::Duration;

/// Why a request came to nothing: never an HTTP status, which is an answer ([`Response::status`]).
#[derive(Debug)]
pub enum Error {
    /// Nothing answered there: the name was not found, or no connection could be made (refused, no route).
    NotThere,
    /// It took longer than the client waits.
    TimedOut,
    /// The connection broke after it was made, or the request could not be written to it.
    Broken(String),
    /// What came back could not be read as asked: cut short, too large, or not the JSON expected.
    Unreadable(String),
    /// A typed body could not be written as JSON.
    NotWritten(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotThere => f.write_str("nothing answered there"),
            Error::TimedOut => f.write_str("it did not answer in time"),
            Error::Broken(why) => write!(f, "the connection broke: {why}"),
            Error::Unreadable(why) => write!(f, "the answer could not be read: {why}"),
            Error::NotWritten(why) => write!(f, "the request could not be written: {why}"),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    /// The library's error, as one of ours. Every way of reaching nothing is [`Error::NotThere`], so no caller
    /// reads the library's kinds.
    fn of(e: ureq::Error) -> Error {
        use std::io::ErrorKind;
        match e {
            ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => Error::NotThere,
            ureq::Error::Io(io) if matches!(io.kind(), ErrorKind::ConnectionRefused | ErrorKind::NotFound | ErrorKind::AddrNotAvailable | ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable) => Error::NotThere,
            ureq::Error::Timeout(_) => Error::TimedOut,
            ureq::Error::Io(io) if io.kind() == ErrorKind::TimedOut => Error::TimedOut,
            ureq::Error::Json(e) => Error::Unreadable(e.to_string()),
            ureq::Error::BodyExceedsLimit(_) | ureq::Error::BodyStalled => Error::Unreadable(e.to_string()),
            // its words are the address itself, key and all
            ureq::Error::BadUri(_) => Error::NotWritten("the address is not one a request can be made to".into()),
            other => Error::Broken(other.to_string()),
        }
    }
    /// This error with nothing in its words of what in `url` can hold a key: its query and its login, each put
    /// as dots wherever the library's words quote them.
    fn without_secrets_of(self, url: &str) -> Error {
        let mut secret: Vec<String> = Vec::new();
        // the query, however the rest of the address reads, and the login and the query as a parser finds them
        secret.extend(url.split_once('?').map(|(_, query)| query.to_string()));
        if let Some(u) = airband_core::links::web(url) {
            // the login with the `@` that ends it, as the address writes it: never a bare name found elsewhere
            if !u.username().is_empty() || u.password().is_some() {
                secret.push(format!("{}{}@", u.username(), u.password().map(|p| format!(":{p}")).unwrap_or_default()));
            }
            secret.extend(u.query().map(str::to_string));
        }
        secret.retain(|s| !s.is_empty());
        secret.sort_by_key(|s| std::cmp::Reverse(s.len()));
        let clean = |words: String| secret.iter().fold(words, |w, s| w.replace(s.as_str(), "\u{2022}\u{2022}\u{2022}"));
        match self {
            Error::Broken(w) => Error::Broken(clean(w)),
            Error::Unreadable(w) => Error::Unreadable(clean(w)),
            Error::NotWritten(w) => Error::NotWritten(clean(w)),
            other => other,
        }
    }
}

/// What a request got back: its status, its headers and its body, whatever the status.
pub struct Response {
    status: u16,
    headers: ureq::http::HeaderMap,
    body: Body,
}

/// An HTTP status, as a service answered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status(u16);

impl Status {
    pub fn as_u16(self) -> u16 {
        self.0
    }
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response").field("status", &self.status).finish_non_exhaustive()
    }
}

impl Response {
    fn of(res: ureq::http::Response<ureq::Body>) -> Response {
        let (parts, body) = res.into_parts();
        Response { status: parts.status.as_u16(), headers: parts.headers, body: Body(body) }
    }
    pub fn status(&self) -> Status {
        Status(self.status)
    }
    /// The header of this name, when it is there and is text.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
    /// Whether a header of this name is there at all.
    pub fn has_header(&self, name: &str) -> bool {
        self.headers.contains_key(name)
    }
    pub fn body_mut(&mut self) -> &mut Body {
        &mut self.body
    }
    pub fn into_body(self) -> Body {
        self.body
    }
}

/// What came back after the status and headers, read once, in one of these ways.
pub struct Body(ureq::Body);

impl Body {
    /// Read whole, as JSON of this type.
    pub fn read_json<T: serde::de::DeserializeOwned>(&mut self) -> Result<T, Error> {
        self.0.read_json().map_err(Error::of)
    }
    /// Read whole, as bytes (up to the library's limit, 10 MB).
    pub fn read_to_vec(&mut self) -> Result<Vec<u8>, Error> {
        self.0.read_to_vec().map_err(Error::of)
    }
    /// Read whole, as text.
    pub fn read_to_string(&mut self) -> Result<String, Error> {
        self.0.read_to_string().map_err(Error::of)
    }
    /// Read as it comes, with no limit: for a download or a stream.
    pub fn as_reader(&mut self) -> impl std::io::Read + '_ {
        self.0.as_reader()
    }
    /// The same, owning the body.
    pub fn into_reader(self) -> impl std::io::Read + Send + 'static {
        self.0.into_reader()
    }
}

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
        self.sent(verb, &url, &headers, body).map(Response::of).map_err(|e| Error::of(e).without_secrets_of(&url))
    }
    fn sent(&self, verb: Verb, url: &str, headers: &[(String, String)], body: Option<Vec<u8>>) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
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
            Verb::Get => without_body!(self.agent.get(url)),
            Verb::Delete => without_body!(self.agent.delete(url)),
            Verb::Post => with_body!(self.agent.post(url)),
            Verb::Put => with_body!(self.agent.put(url)),
            Verb::Patch => with_body!(self.agent.patch(url)),
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

/// One request, made before it is sent: where, how, with which headers and what body. Its debug form names the
/// headers and never their values, which can be a key.
pub struct Ask {
    verb: Verb,
    url: String,
    headers: Vec<(String, String)>,
    /// The bytes to send, none for an empty body; an error when a typed body could not be written.
    body: Result<Option<Vec<u8>>, Error>,
}

impl std::fmt::Debug for Ask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let named: Vec<&str> = self.headers.iter().map(|(k, _)| k.as_str()).collect();
        let body = self.body.as_ref().map(|b| b.as_ref().map(Vec::len));
        f.debug_struct("Ask").field("verb", &self.verb).field("url", &self.url.split_once('?').map_or(self.url.as_str(), |(at, _)| at)).field("headers", &named).field("body_bytes", &body).finish()
    }
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
        let bytes = serde_json::to_vec(body).map(Some).map_err(|e| Error::NotWritten(e.to_string()));
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

#[cfg(test)]
mod tests {
    #[test]
    fn an_error_never_carries_a_key_from_the_address() {
        // A-22: the library's words for a bad address quote it whole
        let client = super::Client::new(std::time::Duration::from_secs(2));
        for url in ["http://exa mple.test/v1?key=SECRET-1", "http://user:SECRET-2@127.0.0.1:9/v1?key=SECRET-3", "ht!tp://nowhere?key=SECRET-4"] {
            if let Err(e) = client.send(super::Ask::get(url)) {
                let (said, debug) = (e.to_string(), format!("{e:?}"));
                assert!(!said.contains("SECRET") && !debug.contains("SECRET"), "{said} / {debug}");
            }
        }
        let scrubbed = super::Error::Broken("GET http://u:pw@h/x?key=abc failed".into()).without_secrets_of("http://u:pw@h/x?key=abc");
        assert_eq!(scrubbed.to_string(), "the connection broke: GET http://\u{2022}\u{2022}\u{2022}h/x?\u{2022}\u{2022}\u{2022} failed");
    }

    use super::*;
    use std::io::{Read, Write};

    /// A server on this machine that answers one request with `answer` (a whole HTTP response) and hands back
    /// what it was sent.
    fn answering(answer: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let at = format!("http://{}", listener.local_addr().unwrap());
        let served = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            // the head and the body may come in pieces: read until the head is whole and the body it announces
            let mut got = Vec::new();
            let mut piece = [0u8; 4096];
            loop {
                let n = conn.read(&mut piece).unwrap();
                got.extend_from_slice(&piece[..n]);
                let text = String::from_utf8_lossy(&got).to_lowercase();
                let whole = text.split_once("\r\n\r\n").is_some_and(|(head, body)| {
                    let wants = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                    body.len() >= wants
                });
                if n == 0 || whole {
                    break;
                }
            }
            conn.write_all(answer.as_bytes()).unwrap();
            String::from_utf8_lossy(&got).to_string()
        });
        (at, served)
    }

    #[test]
    fn an_answer_is_read_through_this_modules_own_types_whatever_its_status() {
        let (at, served) = answering("HTTP/1.1 409 Conflict\r\ncontent-type: application/json\r\nx-airband-request: r1\r\ncontent-length: 11\r\nconnection: close\r\n\r\n{\"n\": 7}   ");
        let mut res = Client::new(Duration::from_secs(5)).send(Ask::post(format!("{at}/v1/x")).header("authorization", "Bearer secret").json(&[1, 2])).unwrap();
        assert_eq!(res.status().as_u16(), 409);
        assert_eq!((res.header("x-airband-request"), res.has_header("x-airband-kept"), res.header("nope")), (Some("r1"), false, None));
        assert_eq!(res.body_mut().read_json::<serde_json::Value>().unwrap()["n"], 7);
        let sent = served.join().unwrap();
        assert!(sent.starts_with("POST /v1/x ") && sent.contains("authorization: Bearer secret") && sent.ends_with("[1,2]"), "{sent}");
    }

    #[test]
    fn a_place_where_nothing_listens_is_not_there_and_an_unreadable_answer_says_so() {
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        assert!(matches!(Client::new(Duration::from_secs(5)).send(Ask::get(format!("http://{free}/"))), Err(Error::NotThere)));
        let (at, _served) = answering("HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nnot j");
        let mut res = Client::new(Duration::from_secs(5)).send(Ask::get(at)).unwrap();
        assert!(matches!(res.body_mut().read_json::<serde_json::Value>(), Err(Error::Unreadable(_))));
    }

    #[test]
    fn a_request_written_out_for_debugging_names_its_headers_and_never_their_values() {
        let ask = Ask::post("https://example.test/v1/a?key=k-123").header("authorization", "Bearer sk-secret").bytes("text/plain", b"his words".to_vec());
        let shown = format!("{ask:?}");
        assert!(shown.contains("authorization") && shown.contains("/v1/a"), "{shown}");
        for secret in ["sk-secret", "k-123", "his words"] {
            assert!(!shown.contains(secret), "{shown}");
        }
    }
}
