//! Whether a catalogue is taken: its signature, and what it may and may not change in the copy in hand. Whether
//! it is well formed is `form`.
use super::*;

/// What became of a copy that was offered.
#[derive(Debug, Default, PartialEq)]
pub struct Taken {
    /// It is now the one in use.
    pub newer: bool,
    /// What it holds that this build cannot read, a few words each: for a line in a log.
    pub left_out: Vec<String>,
    /// Something worth a line in a log about a copy that was not taken.
    pub note: Option<String>,
}

/// A new offer of a provider this build knows, reached exactly as one of its offers is, and by nothing that
/// runs a program: the one kind of new offer a later copy may add.
fn known_way(known: &Provider, o: &Offer) -> bool {
    !BUILT_IN_ONLY.contains(&o.by.interface.as_str()) && known.offers.iter().any(|was| was.by == o.by && reach(was) == reach(o))
}

/// How this build starts a coding tool, by its id: its launchers and what its row tells the code. Nothing for
/// a tool this build does not know.
fn own_start(built: &Catalog, id: &str) -> (Vec<crate::tools::Launcher>, BTreeMap<String, Value>) {
    built.tools.iter().find(|b| b.id == id).map(|b| (b.launchers.clone(), b.with.clone())).unwrap_or_default()
}

/// Take a newer copy of the catalogue. `signature` is the 64 bytes of its Ed25519 signature, in base64.
/// An error when it is not signed by airband's key, does not read, has run out, or would do something a
/// later copy may not; then nothing changes.
///
/// The copy taken replaces the one in use ([`current`]); whoever is part way through reading the one
/// before finishes with that.
pub fn accept(document: &[u8], signature: &str) -> Result<Taken, String> {
    signed(document, signature)?;
    let text = std::str::from_utf8(document).map_err(|e| e.to_string())?;
    take(&CURRENT, text, crate::clock::now_secs())
}

/// Weigh a copy against the one in use in `slot` and put it in its place, in one hold of the lock: two copies
/// taken at once (the kept one at start and a fetched one) can never leave the older in use.
pub(super) fn take(slot: &RwLock<Option<Arc<Catalog>>>, text: &str, now: u64) -> Result<Taken, String> {
    let mut slot = slot.write().unwrap_or_else(|e| e.into_inner());
    let in_use = slot.get_or_insert_with(|| built_shared().clone()).clone();
    let (newer, taken) = weigh(text, now, &in_use)?;
    if let Some(newer) = newer {
        *slot = Some(Arc::new(newer));
    }
    Ok(taken)
}

/// Where airband's own on-device models are published: only those may be offered by a later copy.
const MODELS_HOST: &str = "airband.dev";
const MODELS_PATH: &str = "/models/";

/// Is this document signed by airband's catalogue key?
pub fn signed(document: &[u8], signature: &str) -> Result<(), String> {
    let key = VerifyingKey::from_bytes(&SIGNED_BY).map_err(|e| e.to_string())?;
    let sig = crate::encode::unbase64(signature.trim()).ok_or("the catalog's signature does not read")?;
    let sig = Signature::from_slice(&sig).map_err(|_| "the catalog's signature does not read")?;
    key.verify(document, &sig).map_err(|_| "the catalog is not signed by airband".to_string())
}

/// A document about to be published, read and checked the way a build would on taking it: it reads, it is
/// well formed, and it says when it runs out.
pub fn inspect(text: &str) -> Result<Catalog, String> {
    let c = read(text)?;
    check(&c)?;
    if hash_of(text)? != c.hash {
        return Err("the hash it carries is not the hash of its rows".into());
    }
    match serde_json::from_str::<Value>(text).ok().and_then(|v| v["expires"].as_u64()) {
        Some(until) if until > crate::clock::now_secs() => Ok(c),
        _ => Err("it does not say when it runs out, or already has".into()),
    }
}

/// Would a build that has `before` compiled in take `newer`? For checking a document against the copy
/// published before it.
pub fn would_take(newer: &str, before: &str) -> Result<(), String> {
    let before = read(before)?;
    match weigh_against(newer, crate::clock::now_secs(), &before, &before)? {
        (Some(_), _) => Ok(()),
        (None, _) => Err(format!("its rev is not higher than the one before ({})", before.rev)),
    }
}

/// Everything about taking a copy but the signature: is it newer than `have`, in date, and within what a
/// later copy may do? The catalogue to use from now on, when it is.
pub(super) fn weigh(text: &str, now: u64, have: &Catalog) -> Result<(Option<Catalog>, Taken), String> {
    weigh_against(text, now, have, built())
}

/// The same, held to what `built` has in place of this build's own catalogue.
pub(super) fn weigh_against(text: &str, now: u64, have: &Catalog, built: &Catalog) -> Result<(Option<Catalog>, Taken), String> {
    let expires = serde_json::from_str::<Value>(text).map_err(|e| format!("the catalog does not read: {e}"))?["expires"].as_u64().ok_or("this copy of the catalog does not say until when it may be taken")?;
    // a signed copy says until when it may be taken, so one that was replaced long ago cannot be served again
    if expires <= now {
        return Err("this copy of the catalog has run out".into());
    }
    let mut newer = read(text)?;
    if newer.rev < have.rev {
        return Ok((None, Taken::default()));
    }
    if newer.rev == have.rev {
        let note = (newer.hash != have.hash).then(|| format!("a catalog with the same rev ({}) has different rows; the one in hand is kept", have.rev));
        return Ok((None, Taken { note, ..Taken::default() }));
    }
    // A provider this build has never heard of may not bring a program to run: such an offer is left out, so
    // the rest of the copy can still be taken.
    for p in newer.providers.iter_mut().filter(|p| !built.providers.iter().any(|b| b.id == p.id)) {
        let before = p.offers.len();
        Arc::make_mut(p).offers.retain(|o| !BUILT_IN_ONLY.contains(&o.by.interface.as_str()));
        if p.offers.len() < before {
            newer.left_out.push(format!("{}: an offer that would run a program", p.id));
        }
    }
    // An offer new to a provider this build knows is kept only when it is reached exactly as one of that
    // provider's compiled-in offers is: the same interface told the same things, at the same address. That
    // is a new model of a service he may already have given a key for. Anything else would send that key
    // somewhere this build was never told about, and is left out. So is any new offer that runs a program,
    // even one worded exactly as a compiled-in offer is.
    for p in newer.providers.iter_mut() {
        let Some(known) = built.providers.iter().find(|b| b.id == p.id) else { continue };
        let before = p.offers.len();
        Arc::make_mut(p).offers.retain(|o| known.offers.iter().any(|was| was.id == o.id) || known_way(known, o));
        if p.offers.len() < before {
            newer.left_out.push(format!("{}: a new offer reached another way than this build knows it by", p.id));
        }
    }
    newer.servers = built.servers.clone();
    newer.links = built.links.clone();
    newer.secrets = built.secrets.clone();
    newer.terminals = built.terminals.clone();
    newer.frames = built.frames.clone();
    newer.installers = built.installers.clone();
    // How a coding tool is started is a program to run, and is this build's own for every tool it knows. What
    // a later copy says about that is not kept: a tool new to this build comes with no way to start it.
    for t in newer.tools.iter_mut() {
        (t.launchers, t.with) = own_start(built, &t.id);
    }
    if newer.providers.is_empty() {
        return Err("the catalog has no providers in it".into());
    }
    // A provider or tool this build has compiled in is never lost by taking a copy: when the copy does not list it
    // (or lists it in a shape this build cannot read) and does not say it is retired, the compiled-in row
    // stays. What he has chosen keeps working.
    for known in &built.providers {
        if !newer.providers.iter().any(|p| p.id == known.id) && !newer.retired.contains(&known.id) {
            newer.left_out.push(format!("{}: not in this copy in a shape this build reads; kept as compiled in", known.id));
            newer.providers.push(known.clone());
        }
    }
    // the same for a coding tool this build knows: his agents of it are still heard
    for known in &built.tools {
        if !newer.tools.iter().any(|t| t.id == known.id) && !newer.retired.contains(&known.id) {
            newer.left_out.push(format!("{}: not in this copy in a shape this build reads; kept as compiled in", known.id));
            newer.tools.push(known.clone());
        }
    }
    check(&newer)?;
    allowed(&newer, built)?;
    let left_out = newer.left_out.clone();
    Ok((Some(newer), Taken { newer: true, left_out, note: None }))
}

/// What a copy taken later may not do, whoever signed it. The signature says it came from airband; these say
/// that even airband's own key cannot, through a catalogue, run a different program on his computer or send
/// a key he has already entered somewhere new. Those take a new build, which he installs himself.
///
/// - An offer this build knows (by its provider and its own id) keeps the job it does, the interface it is
///   reached through and everything that interface is told (`by`, which includes how a key is presented),
///   and the usual of every setting that says where it is reached (`reach`). So a row that changes how a
///   known service is called is refused, not read as the old call.
/// - An offer new to a provider this build knows is reached exactly as one of its known offers is, or it is
///   not there ([`weigh`] leaves the others out). So nothing that runs a program ([`BUILT_IN_ONLY`]) is
///   added to a known provider, and a key he gave is sent nowhere new. A provider this build does not know
///   cannot bring a program either.
/// - Whether a known provider takes a key does not change, nor where and how its key is sent to be tested.
/// - A version of a model this build knows keeps its files. New files are a new version.
/// - A model's files come from airband's address.
pub(super) fn allowed(newer: &Catalog, built: &Catalog) -> Result<(), String> {
    for p in &newer.providers {
        let Some(known) = built.providers.iter().find(|b| b.id == p.id) else { continue };
        for o in &p.offers {
            match known.offers.iter().find(|x| x.id == o.id) {
                Some(was) => {
                    if o.by != was.by || o.job != was.job {
                        return Err(format!("the catalog would change how {} is reached", p.id));
                    }
                    if reach(o) != reach(was) {
                        return Err(format!("the catalog would change where {} is reached", p.id));
                    }
                }
                None if !known_way(known, o) => return Err(format!("the catalog would have {} reached a new way", p.id)),
                None => {}
            }
        }
        if known.key != p.key {
            return Err(format!("the catalog would change whether {} takes a key", p.id));
        }
        // testing a key sends it: where and how is this build's own for every provider it knows
        if known.check != p.check {
            return Err(format!("the catalog would change where a key for {} is sent to be tested", p.id));
        }
    }
    // the same for how a coding tool is started: only what this build came with
    for t in &newer.tools {
        if (t.launchers.clone(), t.with.clone()) != own_start(built, &t.id) {
            return Err(format!("the catalog would change the program run to start {}", t.id));
        }
    }
    for m in &newer.models {
        // fetched from airband's own site, as the address is built: its host and its path, read by the rules for
        // web addresses
        let ours = crate::links::web(&m.base).is_some_and(|u| u.scheme() == "https" && u.host_str() == Some(MODELS_HOST) && u.path().starts_with(MODELS_PATH));
        if !ours {
            return Err(format!("the catalog names a model that is not airband's to offer: {}", m.kind));
        }
        if built.models.iter().any(|b| b.kind == m.kind && b.version == m.version && b.files != m.files) {
            return Err(format!("the catalog would change the files of {} without a new version", m.kind));
        }
    }
    Ok(())
}

/// Where an offer is reached: the usual of each of its settings that says so.
pub(super) fn reach(o: &Offer) -> Vec<(String, String)> {
    o.asks.iter().filter(|a| a.reach).map(|a| (a.field.clone(), a.usual.clone())).collect()
}
