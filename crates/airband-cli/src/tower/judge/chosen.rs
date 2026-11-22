//! Choosing what reads for the tower from a radio, and the list a radio chooses from. What reads now and its
//! last failure are the settings' ([`super::super::settings::Settings`]).
use super::*;
use crate::providers::Job;

/// Choose what reads, asked for from a radio. Only a provider he could choose in `airband setup`, reached the
/// way its row says: a radio cannot name a command to run here. It is tried with a real call before anything
/// is kept. The key is kept encrypted with the others. Returns the provider's name.
pub(in crate::tower) async fn set_reader(settings: &Settings, wanted: airband_core::radio::SetReads) -> Result<String, String> {
    use crate::providers::{Job, Use};
    // one of this tower's own list, named by its provider and its own id
    let (p, offer) = crate::providers::in_order(Job::Summary).into_iter().find(|(p, o)| p.id == wanted.kind && o.id == wanted.offer).ok_or("That is not something that can be chosen to read here.")?;
    if let Some(why) = settings.unread() {
        return Err(format!("This computer's settings cannot be read right now: {why}"));
    }
    let cfg = settings.current();
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
    let keyed = p.key;
    settings.change(|cfg| {
        if keyed {
            // a provider has one key here: a new one takes the place of the one before
            cfg.keep_key(&p.id, key);
        }
        cfg.summarizer = using.clone();
    })?;
    // it has just answered: the last failure is over
    settings.asked(None);
    // a model this tower was running for the last choice is stopped, unless it is the new one too
    if local_model(&using).is_none() {
        super::local::stop();
    }
    log(format!("a radio chose what reads: {}", offer.label));
    Ok(offer.label.to_string())
}

/// What can read on this tower, for a radio that offers the choice: the tower's own list for its own job, so
/// a phone with another copy of the catalogue offers nothing this tower would refuse.
pub(in crate::tower) fn offered(settings: &Settings) -> airband_core::radio::ReadsOffered {
    use airband_core::radio::{ReadsAsk, ReadsOffer, ReadsOffered};
    let cfg = settings.current();
    let now = &cfg.summarizer;
    let kept: BTreeSet<&str> = cfg.keys.values().map(|k| k.provider.as_str()).collect();
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
            has_key: p.key && kept.contains(p.id.as_str()),
            key_name: p.key_name.clone(),
            keys_at: p.keys_at.clone(),
            asks: o.asks.iter().map(|a| ReadsAsk { field: a.field.clone(), question: a.question.clone(), usual: a.usual.clone() }).collect(),
        })
        .collect();
    ReadsOffered { now: (!now.is_none()).then(|| airband_core::radio::OfferName { provider: now.kind.clone(), offer: now.offer.clone() }), failed: settings.failed(), offers }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_just_chosen_is_offered_with_its_key_kept_and_no_old_failure() {
        let cfg: Config = serde_json::from_str(r#"{"token":"own","radio_token":"radio"}"#).unwrap();
        let (p, o) = crate::providers::in_order(Job::Summary).into_iter().find(|(p, _)| p.key).expect("a provider with a key");
        let settings = Settings::held(cfg);
        settings.asked(Some(Trouble::Unreachable));
        assert_eq!(offered(&settings).failed, Some(Trouble::Unreachable));
        // what set_reader does once the reader has answered
        settings
            .change(|cfg| {
                cfg.keep_key(&p.id, "a-key".into());
                cfg.summarizer = crate::providers::Use { kind: p.id.clone(), offer: o.id.clone(), key: "a-key".into(), set: Default::default() };
            })
            .unwrap();
        settings.asked(None);
        let now = offered(&settings);
        assert_eq!(now.failed, None);
        assert!(now.offers.iter().filter(|o| o.provider == p.id).all(|o| o.has_key), "a key kept is offered at once");
        assert_eq!(now.now.map(|n| n.provider), Some(p.id.clone()));
    }
}
