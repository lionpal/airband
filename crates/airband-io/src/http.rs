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
