//! The HTTP agent everything here uses, with a second way to find an address.
//!
//! Some networks answer every DNS question themselves, from their own cache, whichever server was asked. After a
//! domain changes hands they go on giving the old answer for hours: this happened to the relay's own name on the
//! network it was built on. So when the system cannot resolve a name, the name is asked for over HTTPS from a
//! public resolver reached by its address, which such a network cannot rewrite. The system is always asked first.
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

/// Public resolvers that answer over HTTPS at a fixed address, with a certificate issued for that address.
const RESOLVERS: [&str; 2] = ["https://1.1.1.1/dns-query", "https://8.8.8.8/resolve"];
const REMEMBER: Duration = Duration::from_secs(300);

#[derive(Default)]
struct Fallback {
    system: DefaultResolver,
    found: Mutex<HashMap<String, (Instant, Vec<IpAddr>)>>,
}

impl std::fmt::Debug for Fallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Fallback")
    }
}

impl Fallback {
    fn over_https(&self, host: &str) -> Vec<IpAddr> {
        if let Some((when, ips)) = self.found.lock().unwrap().get(host) {
            if when.elapsed() < REMEMBER {
                return ips.clone();
            }
        }
        // a plain agent: these addresses need no lookup, so this cannot come back here
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(6))).build().into();
        for resolver in RESOLVERS {
            let answer = agent.get(format!("{resolver}?name={host}&type=A")).header("accept", "application/dns-json").call().ok().and_then(|mut r| r.body_mut().read_json::<serde_json::Value>().ok());
            let ips: Vec<IpAddr> = answer
                .iter()
                .flat_map(|a| a["Answer"].as_array().cloned().unwrap_or_default())
                .filter(|a| a["type"] == 1)
                .filter_map(|a| a["data"].as_str().and_then(|d| d.parse().ok()))
                .collect();
            if !ips.is_empty() {
                self.found.lock().unwrap().insert(host.to_string(), (Instant::now(), ips.clone()));
                return ips;
            }
        }
        Vec::new()
    }
}

impl Resolver for Fallback {
    fn resolve(&self, uri: &Uri, config: &Config, timeout: NextTimeout) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let failed = match self.system.resolve(uri, config, timeout) {
            Ok(found) => return Ok(found),
            Err(e) => e,
        };
        let Some(host) = uri.host() else { return Err(failed) };
        let port = uri.port_u16().unwrap_or(if uri.scheme_str() == Some("http") { 80 } else { 443 });
        let mut out = self.empty();
        out.truncate(0);
        for ip in self.over_https(host).into_iter().take(8) {
            out.push(SocketAddr::new(ip, port));
        }
        if out.len() == 0 { Err(failed) } else { Ok(out) }
    }
}

/// The same, for an address that may not be there at all (his computer's, on a network he may have left): it
/// gives up on making the connection after `connect`, so finding that out costs a moment and not a minute.
pub fn agent_quick(timeout: Duration, connect: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder().timeout_global(Some(timeout)).timeout_connect(Some(connect)).http_status_as_error(false).build();
    ureq::Agent::with_parts(config, DefaultConnector::default(), Fallback::default())
}

/// An agent that gives up after `timeout`, treats every HTTP status as an answer and not an error, and can find a
/// host the local network's DNS has wrong.
pub fn agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false).build();
    ureq::Agent::with_parts(config, DefaultConnector::default(), Fallback::default())
}
