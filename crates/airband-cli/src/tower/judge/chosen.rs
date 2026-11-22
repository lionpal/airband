//! What reads for the tower now: the one chosen from a radio since it started, the last failure, the providers a
//! key has been kept for, and the list a radio chooses from.
use super::*;
use crate::providers::Job;

/// What reads, when he has changed it from a radio since this tower started: used from then on, with no
/// restart. The settings file has it too, for the next start.
pub(super) static READER: Mutex<Option<Summarizer>> = Mutex::new(None);

/// What reads for the tower now.
pub(in crate::tower) fn reader(cfg: &Config) -> Summarizer {
    READER.lock().unwrap().clone().unwrap_or_else(|| cfg.summarizer.clone())
}

/// Choose what reads, asked for from a radio. Only a provider he could choose in `airband setup`, reached the
/// way its row says: a radio cannot name a command to run here. It is tried with a real call before anything
/// is kept. The key is kept encrypted with the others. Returns the provider's name.
pub(in crate::tower) async fn set_reader(wanted: airband_core::radio::SetReads) -> Result<String, String> {
    use crate::providers::{Job, Use};
    // one of this tower's own list, named by its provider and its own id
    let (p, offer) = crate::providers::in_order(Job::Summary).into_iter().find(|(p, o)| p.id == wanted.kind && o.id == wanted.offer).ok_or("That is not something that can be chosen to read here.")?;
    let mut cfg = config::read_keyed().ok_or("This computer's settings cannot be read right now.")?;
    let key = match (p.key, wanted.key.trim()) {
        (false, _) => String::new(),
        (true, "") => cfg.key_of(&p.id).ok_or_else(|| format!("{} needs a key.", p.name))?,
        (true, given) => given.to_string(),
    };
    // only the settings its row asks for
    let set = wanted.set.into_iter().filter(|(field, value)| !value.trim().is_empty() && offer.asks.iter().any(|a| a.field == *field)).map(|(f, v)| (f, Value::String(v.trim().to_string()))).collect();
    let using = Use { kind: p.id.clone(), offer: offer.id.clone(), key: key.clone(), set };
    let said = |Failed(trouble, more): Failed| {
        if !more.is_empty() {
            log(format!("trying {} to read: {more}", p.name));
        }
        trouble.said(&offer.label)
    };
    // a model on this computer is fetched the first time it is chosen: this call waits through that, and only
    // this one (every other is told it is not ready yet, and gives the message as written)
    if let Some((model, server)) = local_model(&using) {
        tokio::task::spawn_blocking(move || super::local::ensure(&model, &server, true)).await.map_err(|e| e.to_string())?.map_err(|more| said(Failed(Trouble::Program, more)))?;
    }
    ask(&using, READY.into(), None).await.map_err(said)?;
    if p.key {
        // a provider has one key here: a new one takes the place of the one before
        cfg.keep_key(&p.id, key);
    }
    cfg.summarizer = using.clone();
    config::save(&cfg)?;
    // a model this tower was running for the last choice is stopped, unless it is the new one too
    if local_model(&using).is_none() {
        super::local::stop();
    }
    chosen(using, p.key);
    log(format!("a radio chose what reads: {}", offer.label));
    Ok(offer.label.to_string())
}

/// What reads from now on, just tried and kept: the radios are offered it as chosen, with its key kept when it
/// has one, and the last failure is over, since it has just answered.
fn chosen(using: Summarizer, keyed: bool) {
    answered(&using.kind, keyed);
    *READER.lock().unwrap() = Some(using);
}
fn answered(provider: &str, keyed: bool) {
    if keyed {
        KEYED.lock().unwrap().insert(provider.to_string());
    }
    *FAILED.lock().unwrap() = None;
}

/// The providers a key has been kept for since this tower started, beside the ones its settings had then.
static KEYED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Why what reads could not, the last time it was asked: said to him on his phone, and gone when it next
/// answers. Nothing else is asked in its place.
pub(super) static FAILED: Mutex<Option<Trouble>> = Mutex::new(None);

/// Why what reads could not, the last time it was asked. Nothing while it works.
pub(in crate::tower) fn failed() -> Option<Trouble> {
    *FAILED.lock().unwrap()
}

/// What can read on this tower, for a radio that offers the choice: the tower's own list for its own job, so
/// a phone with another copy of the catalogue offers nothing this tower would refuse.
pub(in crate::tower) fn offered(cfg: &Config) -> airband_core::radio::ReadsOffered {
    use airband_core::radio::{ReadsAsk, ReadsOffer, ReadsOffered};
    let now = reader(cfg);
    let kept: BTreeSet<String> = cfg.keys.values().map(|k| k.provider.clone()).chain(KEYED.lock().unwrap().iter().cloned()).collect();
    let offers = crate::providers::in_order(Job::Summary)
        .into_iter()
        .map(|(p, o)| ReadsOffer {
            provider: p.id.clone(),
            offer: o.id.clone(),
            label: o.label.clone(),
            about: o.about.clone(),
            price: o.price.clone(),
            preferred: o.preferred,
            free: o.local,
            login: o.login.clone(),
            key: p.key,
            has_key: p.key && kept.contains(&p.id),
            key_name: p.key_name.clone(),
            keys_at: p.keys_at.clone(),
            asks: o.asks.iter().map(|a| ReadsAsk { field: a.field.clone(), question: a.question.clone(), usual: a.usual.clone() }).collect(),
        })
        .collect();
    ReadsOffered { now: (!now.is_none()).then(|| airband_core::radio::OfferName { provider: now.kind.clone(), offer: now.offer.clone() }), failed: failed(), offers }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_just_chosen_is_offered_with_its_key_kept_and_no_old_failure() {
        let cfg: Config = serde_json::from_str(r#"{"token":"own","radio_token":"radio"}"#).unwrap();
        let (p, _) = crate::providers::in_order(Job::Summary).into_iter().find(|(p, _)| p.key).expect("a provider with a key");
        *FAILED.lock().unwrap() = Some(Trouble::Unreachable);
        answered(&p.id, true);
        let now = offered(&cfg);
        assert_eq!(now.failed, None);
        assert!(now.offers.iter().filter(|o| o.provider == p.id).all(|o| o.has_key));
    }
}
