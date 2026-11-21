//! The HTTP agent everything here uses. A name is found the way the system finds it, and no other way: nothing
//! is asked of a resolver he did not choose.
use std::time::Duration;

/// For an address that may not be there at all (his computer's, on a network he may have left): it gives up on
/// making the connection after `connect`, so finding that out costs a moment and not a minute.
pub fn agent_quick(timeout: Duration, connect: Duration) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(timeout)).timeout_connect(Some(connect)).http_status_as_error(false).build().into()
}

/// An agent that gives up after `timeout` and treats every HTTP status as an answer and not an error.
pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false).build().into()
}
