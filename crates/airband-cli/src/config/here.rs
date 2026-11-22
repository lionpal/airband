//! This computer as a phone finds it: its addresses on the local network and on a private network laid over
//! the internet, and the name it goes by.
use super::Config;

/// This computer's addresses on its local network: the private ranges, and nothing a stranger could route to.
pub(crate) fn lan_addrs() -> Vec<std::net::Ipv4Addr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(a) if a.is_private() => Some(a),
            _ => None,
        })
        .collect()
}

/// This machine's addresses in the shared address space, 100.64.0.0/10 (RFC 6598): the range a private
/// network laid over the internet gives its machines. The tower listens on these besides the machine itself.
/// Which product made the network is not known here and not asked.
pub(crate) fn overlay_addrs() -> Vec<std::net::Ipv4Addr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(a) if a.octets()[0] == 100 && (64..128).contains(&a.octets()[1]) => Some(a),
            _ => None,
        })
        .collect()
}

/// This computer's name for a phone paired with several: the configured one, else the short host name.
pub(crate) fn station(cfg: &Config) -> String {
    if let Some(s) = cfg.station.clone().filter(|s| !s.trim().is_empty()) {
        return s;
    }
    // Asked of the system once: this is called for every answer to a radio. A Mac has a name he gave it
    // ("Charles's Mac mini"), which is what he knows it by; elsewhere the host's own name, up to its first dot.
    // The name is used as it is given: nothing is cut out of it.
    static HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let said = |program: &str, args: &[&str]| std::process::Command::new(program).args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|name| !name.is_empty());
    let short = HOST
        .get_or_init(|| {
            let given = if cfg!(target_os = "macos") { said("scutil", &["--get", "ComputerName"]) } else { None };
            given.or_else(|| said("hostname", &[]).map(|host| host.split('.').next().unwrap_or_default().to_string())).unwrap_or_default()
        })
        .clone();
    if short.is_empty() { "computer".into() } else { short }
}
