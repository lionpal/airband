//! The HTTP client everything uses. A name is found the way the system finds it, and no other way: nothing is
//! asked of a resolver he did not choose.
use airband_core::providers::Answering;
use std::time::Duration;

pub use ureq::{Agent, Body, Error};
pub type Response = ureq::http::Response<ureq::Body>;

/// For an address that may not be there at all (his computer's, on a network he may have left): it gives up on
/// making the connection after `connect`, so finding that out costs a moment and not a minute.
pub fn agent_quick(timeout: Duration, connect: Duration) -> Agent {
    Agent::config_builder().timeout_global(Some(timeout)).timeout_connect(Some(connect)).http_status_as_error(false).build().into()
}

/// An agent that gives up after `timeout` and treats every HTTP status as an answer and not an error.
pub fn agent(timeout: Duration) -> Agent {
    Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false).build().into()
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
pub fn call<E: airband_core::radio::wire::Endpoint>(agent: &Agent, base: &str, headers: &[(&str, &str)], id: Option<&str>, query: &E::Query, body: Option<&E::Body>) -> Result<Said<E::Answer>, Error> {
    use airband_core::radio::wire::{Method, Refusal, address};
    let url = format!("{}{}", base.trim_end_matches('/'), address::<E>(id, query));
    let mut res = match E::METHOD {
        Method::Get => headers.iter().fold(agent.get(&url), |r, (k, v)| r.header(*k, *v)).call(),
        Method::Delete => headers.iter().fold(agent.delete(&url), |r, (k, v)| r.header(*k, *v)).call(),
        Method::Post => {
            let req = headers.iter().fold(agent.post(&url), |r, (k, v)| r.header(*k, *v));
            match body {
                Some(body) => req.send_json(body),
                None => req.send_empty(),
            }
        }
    }?;
    let status = res.status().as_u16();
    Ok(match status {
        200 => res.body_mut().read_json::<E::Answer>().map_or(Said::Trouble(status), Said::Took),
        _ => res.body_mut().read_json::<Refusal>().map_or(Said::Trouble(status), Said::No),
    })
}
