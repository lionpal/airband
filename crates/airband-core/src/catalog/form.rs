//! Whether a catalogue is well formed: names that are safe where they are used, files that stay in their
//! folders and come over a secure connection. Every copy is checked this way before it is taken.
use super::*;

/// A name that is safe as a setting, a folder and a part of an address: lower-case letters, digits, `.`, `-`
/// and `_`, starting with a letter or a digit.
pub(super) fn plain(name: &str) -> bool {
    name.bytes().next().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.' | b'_'))
}

/// Is this catalogue well formed? Run on every copy before it is taken, on the one built in by the tests,
/// and on a document about to be published (`airband catalog check`).
pub fn check(c: &Catalog) -> Result<(), String> {
    if c.providers.is_empty() {
        return Err("the catalog has no providers in it".into());
    }
    for p in &c.providers {
        if !plain(&p.id) || crate::providers::Use::reserved(&p.id) {
            return Err(format!("\"{}\" cannot be a provider's id", p.id));
        }
        if c.providers.iter().filter(|o| o.id == p.id).count() != 1 {
            return Err(format!("{} is listed twice", p.id));
        }
        for o in &p.offers {
            // an offer's id sits beside the provider's where a choice is written down
            if !plain(&o.id) || crate::providers::Use::reserved(&o.id) || p.offers.iter().filter(|x| x.id == o.id).count() != 1 {
                return Err(format!("{}: \"{}\" cannot be an offer's id, or is there twice", p.id, o.id));
            }
            if !plain(&o.by.interface) {
                return Err(format!("{}: \"{}\" cannot be an interface's name", p.id, o.by.interface));
            }
            for a in &o.asks {
                // a setting sits beside `kind`, `offer` and `key` where it is written down
                if !plain(&a.field) || crate::providers::Use::reserved(&a.field) || o.asks.iter().filter(|x| x.field == a.field).count() != 1 {
                    return Err(format!("{}: \"{}\" cannot be a setting's name, or is there twice", p.id, a.field));
                }
            }
            if o.voices.iter().any(|v| v.id.is_empty() || v.name.is_empty()) {
                return Err(format!("{}: a voice with no name", p.id));
            }
            // an offer with voices says which one is used until he chooses, and it is one of its own
            if !o.voices.is_empty() && !o.usual("voice").is_some_and(|usual| o.voices.iter().any(|v| v.id == usual)) {
                return Err(format!("{} {}: names no usual voice of its own", p.id, o.id));
            }
        }
    }
    // a coding tool's sign-in writes summaries one way: the one offer its row prefers is what a first run
    // starts him on, so there is never more than one to pick from
    for p in &c.providers {
        for o in p.offers.iter().filter(|o| !o.login.is_empty() && o.preferred && o.job == crate::providers::Job::Summary) {
            if c.providers.iter().flat_map(|q| q.offers.iter()).filter(|x| x.login == o.login && x.preferred && x.job == o.job).count() != 1 {
                return Err(format!("{}: more than one preferred offer writes summaries with this sign-in", o.login));
            }
        }
    }
    // a job has one model to start with at most
    for m in c.models.iter().filter(|m| m.with.get("usual").and_then(|u| u.as_bool()) == Some(true)) {
        if c.models.iter().filter(|o| o.job == m.job && o.with.get("usual").and_then(|u| u.as_bool()) == Some(true)).count() != 1 {
            return Err(format!("{}: another model for its job is marked usual too", m.kind));
        }
    }
    // a model's name, its version and its files' paths are parts of a folder on the device: none may lead
    // out of it
    let part = |name: &str| !name.is_empty() && !name.starts_with('.') && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'));
    for m in &c.models {
        if m.job == crate::providers::Job::Voice && !m.voices.iter().any(|(name, _)| *name == m.usual_voice) {
            return Err(format!("{}: names no usual voice of its own", m.kind));
        }
        if !plain(&m.kind) || !(m.version.is_empty() || plain(&m.version)) || c.models.iter().filter(|o| o.kind == m.kind).count() != 1 {
            return Err(format!("\"{}\" cannot be a model's name, or is there twice", m.kind));
        }
        if m.files.is_empty() || m.files.iter().any(|(path, size, sha)| path.split('/').any(|p| !part(p)) || *size == 0 || !crate::encode::is_hex(sha, 64..=64)) {
            return Err(format!("{}: a file outside its own folder, or one with no size or hash", m.kind));
        }
        if !crate::links::secure(&m.base) {
            return Err(format!("{}: its files are not fetched over https", m.kind));
        }
    }
    // one person to write to, by a name and a number a text message can go to
    match c.contact.as_slice() {
        [one] if !one.name.trim().is_empty() && one.sms.strip_prefix('+').is_some_and(|d| (8..=15).contains(&d.len()) && d.bytes().all(|b| b.is_ascii_digit())) => {}
        _ => return Err("the catalog names no one to write to, more than one, or a number a text cannot go to".into()),
    }
    crate::links::check(&c.links)?;
    check_machines(&c.machines)?;
    for s in &c.servers {
        if !plain(&s.id) || !crate::links::secure(&s.base) || s.archives.iter().any(|a| !part(&a.file) || !crate::encode::is_hex(&a.sha256, 64..=64)) || s.program.split('/').any(|p| !part(p)) {
            return Err(format!("{}: not a program that can be fetched safely", s.id));
        }
    }
    Ok(())
}
