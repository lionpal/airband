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
    // The end-to-end suites that prove the relay path turn these off, so a test machine on such a network still
    // goes through the relay. Only a build with the `stand-ins` feature reads it; the airband he installs never does.
    if crate::adapters::stand_in("AIRBAND_STAND_IN_NO_PRIVATE_NETWORK").is_some() {
        return Vec::new();
    }
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(a) if a.octets()[0] == 100 && (64..128).contains(&a.octets()[1]) => Some(a),
            _ => None,
        })
        .collect()
}

/// Where a phone may reach this computer straight, in the one order every list of them is in: its private
/// network's addresses first, then (with `lan`) its own network's. With each, whether only a paired radio's
/// sealed requests are served there: on its own network, anyone on it could knock.
pub(crate) fn direct_addrs(lan: bool) -> Vec<(std::net::Ipv4Addr, bool)> {
    overlay_addrs().into_iter().map(|a| (a, false)).chain(lan_addrs().into_iter().filter(|_| lan).map(|a| (a, true))).collect()
}

/// The address a phone is given for one of them.
pub(crate) fn direct_url(addr: std::net::Ipv4Addr, port: u16) -> String {
    format!("http://{addr}:{port}")
}

/// This computer's name for a phone paired with several: the configured one, else the short host name. Empty
/// when the system gives none: a phone knows each computer by its id, and shows no name rather than a made-up one.
pub(crate) fn station(cfg: &Config) -> String {
    if let Some(s) = cfg.station.clone().filter(|s| !s.trim().is_empty()) {
        return s;
    }
    // Asked of the system once: this is called for every answer to a radio. A computer that keeps a name he
    // gave it has that asked for; elsewhere the system's own short host name (`hostname -s`). Each is used as
    // the system gives it: nothing is cut out of it here.
    static HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let said = |program: &str, args: &[&str]| std::process::Command::new(program).args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|name| !name.is_empty());
    HOST.get_or_init(|| {
        let given = if cfg!(target_os = "macos") { said(crate::platform::SCUTIL, &["--get", "ComputerName"]) } else { None };
        given.or_else(|| said(crate::platform::HOSTNAME, &["-s"])).unwrap_or_default()
    })
    .clone()
}

/// The identifier of this computer's model as its system gives it (`hw.model`, "Mac16,10"): the phone shows the
/// computer's shape by it. Empty where the system gives none. Asked of the system once.
pub(crate) fn model() -> String {
    static MODEL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    MODEL.get_or_init(hw_model).clone()
}

#[cfg(target_os = "macos")]
fn hw_model() -> String {
    let name = c"hw.model";
    let mut size: libc::size_t = 0;
    // SAFETY: the name is a C string; asked first for the length, then into a buffer of that length
    if unsafe { libc::sysctlbyname(name.as_ptr(), std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) } != 0 || size == 0 {
        return String::new();
    }
    let mut buf = vec![0u8; size];
    // SAFETY: `buf` holds `size` bytes, and `size` says so
    if unsafe { libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) } != 0 {
        return String::new();
    }
    buf.truncate(size);
    std::ffi::CStr::from_bytes_until_nul(&buf).ok().and_then(|c| c.to_str().ok()).map(str::to_string).unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
fn hw_model() -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_direct_addresses_are_the_private_networks_first_and_the_own_network_only_when_allowed() {
        let all = super::direct_addrs(true);
        let first_own = all.iter().position(|(_, own)| *own).unwrap_or(all.len());
        assert!(all[first_own..].iter().all(|(_, own)| *own), "the private network's come first");
        assert!(super::direct_addrs(false).iter().all(|(_, own)| !own), "none of its own network's without lan");
        assert_eq!(super::direct_url(std::net::Ipv4Addr::new(100, 64, 0, 1), 7800), "http://100.64.0.1:7800");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn this_mac_gives_its_models_identifier() {
        let model = super::model();
        assert!(model.contains(',') && model.bytes().next().is_some_and(|b| b.is_ascii_alphabetic()), "{model:?}");
    }
}
