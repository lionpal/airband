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

/// An address from a row, with the settings it names in braces filled in (`https://{region}.example/v1`,
/// `…/models/{model}:generate`). A value goes in only if it is letters, digits, `.`, `-`, `_` or `:`: nothing
/// that could end a host name or a path and lead a key somewhere else. Err says which setting is not that,
/// in words for him.
pub fn fill(template: &str, setting: &dyn Fn(&str) -> String) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some((before, after)) = rest.split_once('{') {
        let (field, after) = after.split_once('}').ok_or("An address in the catalogue is not well formed.")?;
        let value = setting(field).trim().to_string();
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':')) {
            return Err(format!("\"{value}\" is not something the setting {field} can be. It is letters and digits, like the usual one."));
        }
        out.push_str(before);
        out.push_str(&value);
        rest = after;
    }
    out.push_str(rest);
    Ok(out)
}
