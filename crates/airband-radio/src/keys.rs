//! Testing a key when he gives it, before it is kept. The phone asks the service itself, with the one request
//! the provider's row names (`airband_core::providers::Check`): one that costs nothing and that a wrong key
//! fails. What came of it is read from the status alone, by the row's lists, and said to him in the words
//! here. Nothing the service writes back is read, and a key that did not pass is not kept anywhere.
#![allow(clippy::disallowed_methods, reason = "tries a key against its service")]
use airband_core::providers::{Check, KeyCheck, Method, Provider};
use std::collections::BTreeMap;
use std::time::Duration;

/// How a key is tested: [`check`], or a stand-in for it where no service may be called.
pub(crate) type Test = fn(&Check, &str, &BTreeMap<String, String>) -> KeyCheck;

/// In place of a service, for the tests of everything around a key: any key passes but one that says it is
/// broken.
#[cfg(test)]
pub(crate) fn pretend(_: &Check, key: &str, _: &BTreeMap<String, String>) -> KeyCheck {
    if key.contains("broken") { KeyCheck::WrongKey } else { KeyCheck::Good { paid: false } }
}

/// Ask the service whether it takes this key. `settings` are those of the offer he is choosing (an address,
/// a region), as set or as usual. Blocks for one request.
pub(crate) fn check(check: &Check, key: &str, settings: &BTreeMap<String, String>) -> KeyCheck {
    // an address that names a setting he has not filled in cannot be asked
    let url = match check.address(settings) {
        Ok(url) => url,
        Err(bad_setting) => return bad_setting,
    };
    let agent = crate::net::agent(Duration::from_secs(20));
    let with = |name: &str, value: &str| (name.to_string(), value.to_string());
    let headers: Vec<(String, String)> = std::iter::once(with(&check.auth.header, &check.auth.value(key))).chain(check.headers.iter().map(|(k, v)| with(k, v))).collect();
    let answered = match check.method {
        Method::Post => headers.iter().fold(agent.post(&url), |r, (k, v)| r.header(k.as_str(), v.as_str())).send_empty(),
        Method::Get => headers.iter().fold(agent.get(&url), |r, (k, v)| r.header(k.as_str(), v.as_str())).call(),
    };
    match answered {
        Ok(res) => check.read(res.status().as_u16()),
        Err(_) => KeyCheck::Unreachable,
    }
}

/// What he is told after a key passed a test the service charges for.
pub(crate) fn charged(p: &Provider) -> String {
    format!("{} has no free way to test a key, so one small request it charges for was made.", p.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn a_missing_setting_is_not_told_as_no_connection() {
        let set = BTreeMap::new();
        assert_ne!(check(&row("https://{region}.example/models"), "good", &set), KeyCheck::Unreachable);
    }


    /// A service on this machine that answers each request with the status its path names (`/401/models`), and
    /// 200 only when the key it was sent is `good`. Returns its address and what it was sent.
    fn service() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let at = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or_default().to_string();
                let named: Option<u16> = path.trim_start_matches('/').split('/').next().and_then(|s| s.parse().ok());
                let good = request.to_lowercase().contains("x-test-key: pre-good");
                let status = named.unwrap_or(if good { 200 } else { 401 });
                kept.lock().unwrap().push(request);
                let _ = stream.write_all(format!("HTTP/1.1 {status} X\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}").as_bytes());
            }
        });
        (at, seen)
    }

    fn row(url: &str) -> Check {
        serde_json::from_value(serde_json::json!({ "method": "GET", "url": url, "auth": { "header": "x-test-key", "prefix": "pre-" }, "headers": { "x-version": "7" }, "good": [200], "wrong_key": [401], "refused": [402, 403, 429] })).unwrap()
    }

    #[test]
    fn a_key_is_tested_by_the_rows_request_and_read_by_its_status() {
        let (at, seen) = service();
        let set = BTreeMap::from([("url".to_string(), format!("{at}/"))]);
        // the key goes in the header the row names, with its prefix, beside what else the row says to send
        assert_eq!(check(&row("{url}/models"), "good", &set), KeyCheck::Good { paid: false });
        let sent = seen.lock().unwrap().last().cloned().unwrap().to_lowercase();
        assert!(sent.starts_with("get /models ") && sent.contains("x-test-key: pre-good") && sent.contains("x-version: 7"), "{sent}");
        // a key the service does not know, one it will not serve, and an answer the row does not account for
        assert_eq!(check(&row("{url}/models"), "broken", &set), KeyCheck::WrongKey);
        assert_eq!(check(&row("{url}/402/models"), "good", &set), KeyCheck::Refused);
        assert_eq!(check(&row("{url}/429/models"), "good", &set), KeyCheck::Refused);
        assert_eq!(check(&row("{url}/500/models"), "good", &set), KeyCheck::Unexpected { status: 500 });
        // a request the service charges for says so when it passes
        let paid = Check { paid: true, ..row("{url}/models") };
        assert_eq!(check(&paid, "good", &set), KeyCheck::Good { paid: true });
        // POST is sent with nothing in it
        let post = Check { method: Method::Post, ..row("{url}/token") };
        assert_eq!(check(&post, "good", &set), KeyCheck::Good { paid: false });
        assert!(seen.lock().unwrap().last().unwrap().starts_with("POST /token "));
        // nobody there, or a setting the address names that he has not filled in: nothing is known of the key
        assert_eq!(check(&row("http://127.0.0.1:9/models"), "good", &set), KeyCheck::Unreachable);
        // a setting the address names that he has not filled in is said as that, never as no connection
        let unset = check(&row("https://{region}.example/models"), "good", &set);
        assert!(matches!(&unset, KeyCheck::BadSetting { why } if why.contains("region")), "{unset:?}");
        let p = airband_core::catalog::current().providers[0].clone();
        assert!(!unset.said(&p.name).unwrap().contains("connection"));
    }

    #[test]
    fn every_provider_that_takes_a_key_says_how_it_is_tested_and_each_way_it_goes_has_its_own_words() {
        let catalog = airband_core::catalog::current();
        for p in catalog.providers.iter().filter(|p| p.key) {
            let c = p.check.as_ref().unwrap_or_else(|| panic!("{} takes a key and has no check", p.id));
            assert!(!c.good.is_empty() && !c.wrong_key.is_empty(), "{}", p.id);
            // every setting its address names is one each of its offers asks for
            for o in &p.offers {
                // each filled as it usually is, or with a word where the row has no usual (a region)
                let asked: BTreeMap<String, String> = o.asks.iter().map(|a| (a.field.clone(), if a.usual.is_empty() { "x".to_string() } else { a.usual.clone() })).collect();
                assert!(c.address(&asked).is_ok(), "{} {}: the check names a setting this offer does not have", p.id, o.id);
            }
            let all = [KeyCheck::WrongKey, KeyCheck::Refused, KeyCheck::Unreachable, KeyCheck::Unexpected { status: 500 }].map(|o| o.said(&p.name).unwrap());
            assert!(all.iter().all(|a| all.iter().filter(|b| *b == a).count() == 1) && KeyCheck::Good { paid: false }.said(&p.name).is_none());
        }
        assert!(catalog.providers.iter().filter(|p| !p.key).all(|p| p.check.is_none()));
    }

    /// Against the real services, with real keys and with broken ones:
    ///   GEMINI_KEY=… ELEVENLABS_KEY=… AZURE_KEY=… AZURE_REGION=… cargo test -p airband-radio -- --ignored --nocapture real_keys
    /// A provider whose key is not given is tried with a broken key only.
    #[test]
    #[ignore]
    fn real_keys_pass_and_broken_ones_are_told_apart() {
        let catalog = airband_core::catalog::current();
        let region = std::env::var("AZURE_REGION").unwrap_or("eastus".into());
        for p in catalog.providers.iter().filter(|p| p.key) {
            let c = p.check.as_ref().unwrap();
            // each offer's own settings, as usual: an address, a region
            let o = &p.offers[0];
            let mut set: BTreeMap<String, String> = o.asks.iter().map(|a| (a.field.clone(), a.usual.clone())).collect();
            if set.contains_key("region") {
                set.insert("region".into(), region.clone());
            }
            let broken = check(c, "airband-not-a-key-0000000000000000", &set);
            println!("{:<11} broken key: {broken:?}", p.id);
            assert_eq!(broken, KeyCheck::WrongKey, "{}", p.id);
            if let Ok(key) = std::env::var(format!("{}_KEY", p.id.to_uppercase())) {
                let good = check(c, &key, &set);
                println!("{:<11} real key:   {good:?}", p.id);
                assert_eq!(good, KeyCheck::Good { paid: false }, "{}", p.id);
            }
        }
    }
}
