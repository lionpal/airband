//! The catalogue: every service and every on-device model airband can use, as data.
//!
//! Each is a small JSON file under `catalog/` at the top of the repository: one per provider
//! (`catalog/providers/<id>.json`), one per model that runs on a phone (`catalog/models/<kind>.json`), one per
//! coding tool, one per program the tower fetches to run a model. Building this crate puts them together
//! into one document, which is compiled into the tower and into the app, so both know the whole catalogue
//! with no network at all.
//!
//! A newer copy of the same document can be taken later ([`accept`]): a new voice, a new model id or a new
//! price then shows without a new build. A copy is taken only when it is signed by airband's catalogue key,
//! whose public half is below, when its `rev` is higher than the one in hand, when it has not run out
//! (`expires`), and when it stays inside what a later copy may do ([`allowed`], [`check`]).
//!
//! What a row cannot do is add a way of calling a service. Those are code, in a table for each job where the
//! job is done, and a row names one ([`By`]). Every row that reads is kept here; the program that lists or
//! calls leaves out a row naming an interface it has no code for, so an old build reads a new catalogue and
//! shows what it can use.
//!
//! The document has had two forms. Builds from before the interfaces were named read form 1, at
//! `catalog/1/`; it is still written for them from the same rows (`crate::catalog_v1`). This is form 2.
use crate::providers::{Job, Offer, Provider};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::RwLock;

/// The form of the document this build reads.
pub const FORM: u64 = 2;
/// Where the signed copy is kept, by its form.
pub const ADDRESS: &str = "https://airband.dev/catalog/2/catalog.json";

/// The public half of the key that signs the hosted copy.
const SIGNED_BY: [u8; 32] = include!("catalog.key.in");

/// What was put together from `catalog/` when this was built.
const BUILT_IN: &str = include_str!(concat!(env!("OUT_DIR"), "/catalog.json"));

/// Interfaces that start a program on his computer. An offer on one of these is honoured only as this build
/// has it compiled in: a copy taken later can neither bring one nor change one. (The tower's own table marks
/// the same interfaces, and a test there holds the two together.)
pub const BUILT_IN_ONLY: &[&str] = &["command", "on-this-computer"];

pub struct Catalog {
    /// Goes up with every change that is published.
    pub rev: u64,
    /// What the rows hash to: two documents with the same `rev` and different rows are told apart by it.
    pub hash: String,
    pub providers: Vec<Provider>,
    /// The coding tools airband works with.
    pub tools: &'static [Tool],
    pub models: Vec<PhoneModel>,
    /// Programs the tower fetches to run a model on his computer. Always the ones compiled in.
    pub servers: Vec<Server>,
    /// Words of the trade that a recogniser otherwise hears as everyday ones ("sync" as "sink").
    pub phrases: Vec<String>,
    /// Providers that are no more, by id. One this build has compiled in and a later copy neither lists nor
    /// names here is kept as compiled in.
    pub retired: Vec<String>,
    /// What was in the document and could not be read by this build, in a few words each.
    pub left_out: Vec<String>,
}

/// A coding tool: what its agents are known by (`Agent::harness`), and what to call it where he picks the
/// tools he uses.
pub struct Tool {
    pub id: &'static str,
    pub name: &'static str,
    pub maker: &'static str,
    /// How the tower starts its agents, the one to prefer first. Each names a program that is run, so these
    /// are always the ones this build came with: a copy fetched later can change what a tool is called, and
    /// never what is run for it.
    pub launchers: &'static [crate::tools::Launcher],
}

/// A model that runs on the phone itself.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct PhoneModel {
    pub kind: String,
    /// Its maker and its name.
    pub name: String,
    pub job: Job,
    /// The program in the app that runs it. A build offers only the models it has the program for.
    pub engine: String,
    /// Which set of files this is. Each version is kept in a folder of its own on the device, and a version
    /// once published never changes: new files are a new version.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// Where its files are kept: each is fetched from here by its path.
    pub base: String,
    /// Each file: its path under the model's folder, its size, and its SHA-256. A file that does not match is
    /// not kept.
    pub files: Vec<(String, u64, String)>,
    /// For a voice model: the voices he can choose between, the usual one first. Each is a name (its file is
    /// `voices/<name>.safetensors`, and a short clip of it is at `previews/<name>.m4a` beside the model) and
    /// a few words on how it sounds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voices: Vec<(String, String)>,
    /// Who made it and under what licence, for the screen that offers it.
    #[serde(default)]
    pub credit: String,
    /// What its engine is told about this model, so that another model for the same engine is a row and no
    /// code: for a voice, the `rate` it speaks at, its `lang` and its `quant`. And `prepare`: when the
    /// device has work of its own to do before the model first runs, the sentence to show while it does.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub with: std::collections::BTreeMap<String, Value>,
}

impl PhoneModel {
    /// One of the things its engine is told, as text. Empty when the row does not say.
    pub fn told(&self, name: &str) -> String {
        match self.with.get(name) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            _ => String::new(),
        }
    }
}

/// A program the tower fetches once and runs beside itself, for a provider whose model runs on his computer.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Server {
    pub id: String,
    /// Which build of it, as its maker names it.
    pub build: String,
    /// Where its archives are kept: each is fetched from here by its name.
    pub base: String,
    /// The program inside the unpacked archive.
    pub program: String,
    /// One archive for each kind of computer it is published for. One not listed cannot run it yet.
    pub archives: Vec<Archive>,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Archive {
    pub os: String,
    pub arch: String,
    pub file: String,
    pub sha256: String,
}

static CURRENT: RwLock<Option<&'static Catalog>> = RwLock::new(None);

/// The catalogue compiled into this program.
pub fn built() -> &'static Catalog {
    static BUILT: std::sync::OnceLock<Catalog> = std::sync::OnceLock::new();
    BUILT.get_or_init(|| read(BUILT_IN).expect("the catalogue built into this program does not read"))
}

/// The catalogue in use: the newest one accepted, or the one built in.
pub fn current() -> &'static Catalog {
    if let Some(c) = *CURRENT.read().unwrap() {
        return c;
    }
    *CURRENT.write().unwrap().get_or_insert(built())
}

/// The document built into this program, as it would be published. `expires` is when a copy of it stops
/// being taken, in seconds since 1970.
pub fn built_in(expires: u64) -> String {
    let mut doc: Value = serde_json::from_str(BUILT_IN).expect("the catalogue built into this program does not read");
    doc["expires"] = expires.into();
    doc.to_string()
}

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

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Take a newer copy of the catalogue. `signature` is the 64 bytes of its Ed25519 signature, in base64.
/// An error when it is not signed by airband's key, does not read, has run out, or would do something a
/// later copy may not; then nothing changes.
///
/// A copy is kept for as long as the program runs once it is taken (there is one at the start and perhaps
/// one or two a day after), so that everything read from the catalogue can be borrowed for good.
pub fn accept(document: &[u8], signature: &str) -> Result<Taken, String> {
    signed(document, signature)?;
    let text = std::str::from_utf8(document).map_err(|e| e.to_string())?;
    let (newer, taken) = weigh(text, now(), current())?;
    if let Some(newer) = newer {
        *CURRENT.write().unwrap() = Some(Box::leak(Box::new(newer)));
    }
    Ok(taken)
}

/// Is this document signed by airband's catalogue key?
pub fn signed(document: &[u8], signature: &str) -> Result<(), String> {
    let key = VerifyingKey::from_bytes(&SIGNED_BY).map_err(|e| e.to_string())?;
    let sig = crate::audio::base64_decode(signature.trim()).ok_or("the catalogue's signature does not read")?;
    let sig = Signature::from_slice(&sig).map_err(|_| "the catalogue's signature does not read")?;
    key.verify(document, &sig).map_err(|_| "the catalogue is not signed by airband".to_string())
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
        Some(until) if until > now() => Ok(c),
        _ => Err("it does not say when it runs out, or already has".into()),
    }
}

/// Would a build that has `before` compiled in take `newer`? For checking a document against the copy
/// published before it.
pub fn would_take(newer: &str, before: &str) -> Result<(), String> {
    let before = read(before)?;
    match weigh_against(newer, now(), &before, &before)? {
        (Some(_), _) => Ok(()),
        (None, _) => Err(format!("its rev is not higher than the one before ({})", before.rev)),
    }
}

/// Everything about taking a copy but the signature: is it newer than `have`, in date, and within what a
/// later copy may do? The catalogue to use from now on, when it is.
fn weigh(text: &str, now: u64, have: &Catalog) -> Result<(Option<Catalog>, Taken), String> {
    weigh_against(text, now, have, built())
}

/// The same, held to what `built` has in place of this build's own catalogue.
fn weigh_against(text: &str, now: u64, have: &Catalog, built: &Catalog) -> Result<(Option<Catalog>, Taken), String> {
    let expires = serde_json::from_str::<Value>(text).map_err(|e| format!("the catalogue does not read: {e}"))?["expires"].as_u64().unwrap_or(0);
    // a signed copy says until when it may be taken, so one that was replaced long ago cannot be served again
    if expires <= now {
        return Err("this copy of the catalogue has run out".into());
    }
    let mut newer = read(text)?;
    if newer.rev < have.rev {
        return Ok((None, Taken::default()));
    }
    if newer.rev == have.rev {
        let note = (newer.hash != have.hash).then(|| format!("a catalogue with the same rev ({}) has different rows; the one in hand is kept", have.rev));
        return Ok((None, Taken { note, ..Taken::default() }));
    }
    // A provider this build has never heard of may not bring a program to run: such an offer is left out, so
    // the rest of the copy can still be taken.
    for p in newer.providers.iter_mut().filter(|p| !built.providers.iter().any(|b| b.id == p.id)) {
        let before = p.offers.len();
        p.offers.retain(|o| !BUILT_IN_ONLY.contains(&o.by.interface.as_str()));
        if p.offers.len() < before {
            newer.left_out.push(format!("{}: an offer that would run a program", p.id));
        }
    }
    newer.servers = built.servers.clone();
    // How a coding tool is started is a program to run, and is this build's own for every tool it knows. What
    // a later copy says about that is not kept: a tool new to this build comes with no way to start it.
    newer.tools = keep_all(newer.tools.iter().map(|t| Tool { id: t.id, name: t.name, maker: t.maker, launchers: built.tools.iter().find(|b| b.id == t.id).map(|b| b.launchers).unwrap_or_default() }).collect());
    if newer.providers.is_empty() {
        return Err("the catalogue has no providers in it".into());
    }
    // A provider this build has compiled in is never lost by taking a copy: when the copy does not list it
    // (or lists it in a shape this build cannot read) and does not say it is retired, the compiled-in row
    // stays. What he has chosen keeps working.
    for known in &built.providers {
        if !newer.providers.iter().any(|p| p.id == known.id) && !newer.retired.contains(&known.id) {
            newer.left_out.push(format!("{}: not in this copy in a form this build reads; kept as compiled in", known.id));
            newer.providers.push(known.clone());
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
/// - A provider this build knows keeps, for each job it does, the interface it is reached through and
///   everything that interface is told (`by`), and the usual of every setting that says where it is reached
///   (`reach`). So a row that changes how a known service is called is refused, not read as the old call.
/// - Nothing that runs a program ([`BUILT_IN_ONLY`]) is added to a provider this build knows, and one this
///   build does not know cannot bring any ([`weigh`] leaves those out).
/// - Whether a known provider takes a key does not change.
/// - A version of a model this build knows keeps its files. New files are a new version.
/// - A model's files come from airband's address.
fn allowed(newer: &Catalog, built: &Catalog) -> Result<(), String> {
    for p in &newer.providers {
        let Some(known) = built.providers.iter().find(|b| b.id == p.id) else { continue };
        for o in &p.offers {
            match known.offers.iter().find(|x| x.job == o.job) {
                Some(was) => {
                    if o.by != was.by {
                        return Err(format!("the catalogue would change how {} is reached", p.id));
                    }
                    let reach = |x: &Offer| x.asks.iter().filter(|a| a.reach).map(|a| (a.field.clone(), a.usual.clone())).collect::<Vec<_>>();
                    if reach(o) != reach(was) {
                        return Err(format!("the catalogue would change where {} is reached", p.id));
                    }
                }
                None if BUILT_IN_ONLY.contains(&o.by.interface.as_str()) => return Err(format!("the catalogue would have {} run a program", p.id)),
                None => {}
            }
        }
        if known.key != p.key {
            return Err(format!("the catalogue would change whether {} takes a key", p.id));
        }
    }
    // the same for how a coding tool is started: only what this build came with
    for t in newer.tools {
        if t.launchers != built.tools.iter().find(|b| b.id == t.id).map(|b| b.launchers).unwrap_or_default() {
            return Err(format!("the catalogue would change the program run to start {}", t.id));
        }
    }
    for m in &newer.models {
        if !m.base.starts_with("https://airband.dev/models/") {
            return Err(format!("the catalogue names a model that is not airband's to offer: {}", m.kind));
        }
        if built.models.iter().any(|b| b.kind == m.kind && b.version == m.version && b.files != m.files) {
            return Err(format!("the catalogue would change the files of {} without a new version", m.kind));
        }
    }
    Ok(())
}

/// A name that is safe as a setting, a folder and a part of an address: lower-case letters, digits, `.`, `-`
/// and `_`, starting with a letter or a digit.
fn plain(name: &str) -> bool {
    name.bytes().next().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.' | b'_'))
}

/// Is this catalogue well formed? Run on every copy before it is taken, on the one built in by the tests,
/// and on a document about to be published (`airband catalog check`).
pub fn check(c: &Catalog) -> Result<(), String> {
    if c.providers.is_empty() {
        return Err("the catalogue has no providers in it".into());
    }
    for p in &c.providers {
        if !plain(&p.id) || crate::providers::NOT_AN_ID.contains(&p.id.as_str()) {
            return Err(format!("\"{}\" cannot be a provider's id", p.id));
        }
        if c.providers.iter().filter(|o| o.id == p.id).count() != 1 {
            return Err(format!("{} is listed twice", p.id));
        }
        for o in &p.offers {
            if p.offers.iter().filter(|x| x.job == o.job).count() != 1 {
                return Err(format!("{} does one job two ways", p.id));
            }
            if !plain(&o.by.interface) {
                return Err(format!("{}: \"{}\" cannot be an interface's name", p.id, o.by.interface));
            }
            for a in &o.asks {
                // a setting sits beside `kind` and `key` where it is written down
                if !plain(&a.field) || crate::providers::NOT_AN_ID.contains(&a.field.as_str()) || o.asks.iter().filter(|x| x.field == a.field).count() != 1 {
                    return Err(format!("{}: \"{}\" cannot be a setting's name, or is there twice", p.id, a.field));
                }
            }
            if o.voices.iter().any(|v| v.id.is_empty() || v.name.is_empty()) {
                return Err(format!("{}: a voice with no name", p.id));
            }
        }
    }
    // a model's name, its version and its files' paths are parts of a folder on the device: none may lead
    // out of it
    let part = |name: &str| !name.is_empty() && !name.starts_with('.') && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'));
    for m in &c.models {
        if !plain(&m.kind) || !(m.version.is_empty() || plain(&m.version)) || c.models.iter().filter(|o| o.kind == m.kind).count() != 1 {
            return Err(format!("\"{}\" cannot be a model's name, or is there twice", m.kind));
        }
        if m.files.is_empty() || m.files.iter().any(|(path, size, sha)| path.split('/').any(|p| !part(p)) || *size == 0 || sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit())) {
            return Err(format!("{}: a file outside its own folder, or one with no size or hash", m.kind));
        }
        if !m.base.starts_with("https://") {
            return Err(format!("{}: its files are not fetched over https", m.kind));
        }
    }
    for s in &c.servers {
        if !plain(&s.id) || !s.base.starts_with("https://") || s.archives.iter().any(|a| !part(&a.file) || a.sha256.len() != 64) || s.program.split('/').any(|p| !part(p)) {
            return Err(format!("{}: not a program that can be fetched safely", s.id));
        }
    }
    Ok(())
}

// The document as written. A row is the struct it is read into (`Offer`, `PhoneModel`, `Server`), so a file
// says only what is so and nothing is copied field by field. Rows are read one at a time: one this build
// cannot read is left out and counted, and the rest are kept.

#[derive(Deserialize)]
struct Document {
    #[serde(default)]
    form: u64,
    rev: u64,
    #[serde(default)]
    hash: String,
    #[serde(default)]
    providers: Vec<Value>,
    #[serde(default)]
    models: Vec<Value>,
    #[serde(default)]
    tools: Vec<ToolRow>,
    #[serde(default)]
    servers: Vec<Value>,
    #[serde(default)]
    phrases: Vec<String>,
    #[serde(default)]
    retired: Vec<String>,
}

fn yes() -> bool {
    true
}

/// A provider's row, but for its offers, which are read one at a time.
#[derive(Deserialize)]
struct ProviderHead {
    id: String,
    name: String,
    /// Present when it takes a key: what the key is called, and where keys are made.
    #[serde(default)]
    key: Option<KeyRow>,
    #[serde(default = "yes")]
    offered: bool,
    #[serde(default)]
    offers: Vec<Value>,
}

#[derive(Serialize, Deserialize)]
struct ToolRow {
    id: String,
    name: String,
    #[serde(default)]
    maker: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    launchers: Vec<crate::tools::Launcher>,
}

#[derive(Deserialize)]
struct KeyRow {
    name: String,
    #[serde(default)]
    at: String,
}

/// Kept for as long as the program runs: a catalogue is read once at the start and perhaps once more later.
fn keep(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}
fn keep_all<T>(v: Vec<T>) -> &'static [T] {
    Box::leak(v.into_boxed_slice())
}

/// What the rows of a document hash to (`build.rs` writes it into the document; see `catalog_hash.rs`).
pub fn hash_of(document: &str) -> Result<String, String> {
    serde_json::from_str::<Value>(document).map(|doc| content_hash(&doc)).map_err(|e| e.to_string())
}
include!("catalog_hash.rs");

fn read(text: &str) -> Result<Catalog, String> {
    let doc: Document = serde_json::from_str(text).map_err(|e| format!("the catalogue does not read: {e}"))?;
    if doc.form != FORM {
        return Err(format!("this is form {} of the catalogue, and this build reads form {FORM}", doc.form));
    }
    let mut left_out = Vec::new();
    let mut providers = Vec::new();
    for row in doc.providers {
        let named = row["id"].as_str().unwrap_or("a provider with no id").to_string();
        let head: ProviderHead = match serde_json::from_value(row) {
            Ok(head) => head,
            Err(e) => {
                left_out.push(format!("{named}: {e}"));
                continue;
            }
        };
        let mut offers = Vec::new();
        for o in head.offers {
            match serde_json::from_value::<Offer>(o) {
                Ok(o) => offers.push(o),
                Err(e) => left_out.push(format!("{}: an offer ({e})", head.id)),
            }
        }
        let (key, key_name, keys_at) = match head.key {
            Some(k) => (true, k.name, k.at),
            None => (false, String::new(), String::new()),
        };
        providers.push(Provider { id: head.id, name: head.name, key, key_name, keys_at, offered: head.offered, offers });
    }
    let mut models = Vec::new();
    for row in doc.models {
        let named = row["kind"].as_str().unwrap_or("a model with no name").to_string();
        match serde_json::from_value::<PhoneModel>(row) {
            Ok(m) => models.push(m),
            Err(e) => left_out.push(format!("{named}: {e}")),
        }
    }
    let servers = doc.servers.into_iter().filter_map(|row| serde_json::from_value::<Server>(row).ok()).collect();
    let tools = doc
        .tools
        .into_iter()
        .map(|t| {
            // a launcher in a tool's row starts that tool
            let launchers = t.launchers.into_iter().map(|l| crate::tools::Launcher { harness: t.id.clone(), ..l }).collect();
            Tool { id: keep(t.id), name: keep(t.name), maker: keep(t.maker), launchers: keep_all(launchers) }
        })
        .collect();
    Ok(Catalog { rev: doc.rev, hash: doc.hash, providers, tools: keep_all(tools), models, servers, phrases: doc.phrases, retired: doc.retired, left_out })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in document with a change made to it, weighed as a copy offered to this build.
    fn offered(edit: &dyn Fn(&mut Value)) -> Result<(Option<Catalog>, Taken), String> {
        let mut doc: Value = serde_json::from_str(BUILT_IN).unwrap();
        doc["rev"] = (built().rev + 1).into();
        doc["expires"] = (now() + 3600).into();
        edit(&mut doc);
        weigh(&doc.to_string(), now(), built())
    }
    fn row<'a>(doc: &'a mut Value, id: &str) -> &'a mut Value {
        doc["providers"].as_array_mut().unwrap().iter_mut().find(|p| p["id"] == id).unwrap()
    }

    #[test]
    fn the_built_in_catalogue_reads_whole_and_is_well_formed() {
        let c = built();
        assert!(c.left_out.is_empty(), "rows this build cannot read: {:?}", c.left_out);
        check(c).unwrap();
        assert!(c.rev >= 1 && !c.providers.is_empty() && !c.models.is_empty() && !c.servers.is_empty() && !c.phrases.is_empty());
        for p in &c.providers {
            assert!(!p.offers.is_empty(), "{} does nothing", p.id);
            // where a service is reached is marked as that, so a later copy cannot move it
            for o in &p.offers {
                assert!(o.asks.iter().all(|a| a.reach == matches!(a.field.as_str(), "url" | "region")), "{}: a setting that says where it is reached is not marked", p.id);
            }
        }
    }

    #[test]
    fn the_number_goes_up_when_the_rows_change() {
        // `catalog/rev` holds the number and what the rows hashed to when it was given. Rows changed since:
        // run scripts/catalog-stamp.sh, which raises the number and writes the new hash.
        let stamped = include_str!("../../../catalog/rev");
        let mut lines = stamped.lines();
        assert_eq!(lines.next().and_then(|n| n.trim().parse::<u64>().ok()), Some(built().rev));
        assert_eq!(lines.next().map(str::trim), Some(built().hash.as_str()), "the rows under catalog/ changed and catalog/rev did not: run scripts/catalog-stamp.sh");
        assert_eq!(hash_of(BUILT_IN).unwrap(), built().hash);
    }

    #[test]
    fn a_row_this_build_cannot_read_is_left_out_and_said_so_and_the_rest_is_kept() {
        let c = read(
            r#"{"form":2,"rev":9,"providers":[
            {"id":"x","name":"X","offers":[{"job":"voice","by":{"interface":"something-new"},"label":"New"},{"job":"voice","by":"speech","label":"Old shape"},{"job":"ears","by":{"interface":"transcribe"},"label":"Known","preferred":"yes"}]},
            {"id":"y","name":"Y","key":"a new shape of key","offers":[]},
            {"id":"z","name":"Z","offers":[{"job":"summary","by":{"interface":"chat"},"label":"Reads"}]}],
            "models":[{"kind":"m","name":"M"}]}"#,
        )
        .unwrap();
        // an interface nobody has code for still reads: whoever lists or calls leaves it out
        assert_eq!(c.providers.iter().map(|p| (p.id.as_str(), p.offers.iter().map(|o| o.label.as_str()).collect::<Vec<_>>())).collect::<Vec<_>>(), [("x", vec!["New"]), ("z", vec!["Reads"])]);
        // two offers and one provider and one model did not read, and each is named
        assert_eq!(c.left_out.len(), 4, "{:?}", c.left_out);
        assert!(c.left_out[0].starts_with("x: an offer") && c.left_out[2].starts_with("y:") && c.left_out[3].starts_with("m:"));
        // another form of the document is not this build's to read
        assert!(read(r#"{"rev":9,"providers":[]}"#).is_err() && read(r#"{"form":3,"rev":9,"providers":[]}"#).is_err());
    }

    #[test]
    fn a_later_copy_cannot_change_what_is_run_or_where_a_key_goes() {
        // as it is: taken. A new price, a new voice, a new model id, a new provider on a known interface: taken.
        assert!(offered(&|_| {}).unwrap().1.newer);
        assert!(offered(&|d| row(d, "gemini")["offers"][0]["price"] = "About $1".into()).unwrap().1.newer);
        assert!(offered(&|d| row(d, "openai")["offers"][0]["asks"][1]["usual"] = "gpt-7".into()).unwrap().1.newer);
        assert!(offered(&|d| d["providers"].as_array_mut().unwrap().push(serde_json::json!({ "id": "new", "name": "New", "key": { "name": "New key" }, "offers": [{ "job": "summary", "by": { "interface": "chat" }, "label": "New", "asks": [{ "field": "url", "question": "Address", "usual": "https://new.example/v1", "reach": true }] }] }))).unwrap().1.newer);
        // the program the tower runs for summaries
        assert!(offered(&|d| row(d, "claude")["offers"][0]["by"]["with"]["argv"] = serde_json::json!(["sh", "-c", "anything"])).is_err());
        // where a key he has entered is sent: by a setting, or by what the interface is told
        assert!(offered(&|d| row(d, "openai")["offers"][0]["asks"][0]["usual"] = "https://elsewhere.example/v1".into()).is_err());
        assert!(offered(&|d| row(d, "openai")["offers"][0]["asks"][0]["reach"] = false.into()).is_err());
        assert!(offered(&|d| row(d, "elevenlabs")["offers"][0]["by"]["with"]["url"] = "https://elsewhere.example".into()).is_err());
        // how a known service is called (the old call is not made in its place)
        assert!(offered(&|d| row(d, "gemini")["offers"][0]["by"]["with"]["shaped"] = false.into()).is_err());
        assert!(offered(&|d| row(d, "openrouter")["offers"].as_array_mut().unwrap().push(serde_json::json!({ "job": "voice", "by": { "interface": "command", "with": { "argv": ["x"] } }, "label": "Runs" }))).is_err());
        // a provider nobody has seen may not run a program: that offer is left out, and said so
        let (taken, said) = offered(&|d| d["providers"].as_array_mut().unwrap().push(serde_json::json!({ "id": "new", "name": "New", "offers": [{ "job": "summary", "by": { "interface": "command", "with": { "argv": ["x"] } }, "label": "New" }] }))).unwrap();
        assert!(taken.unwrap().providers.iter().find(|p| p.id == "new").unwrap().offers.is_empty() && said.left_out.len() == 1);
        // the program fetched to run a model is always the one compiled in
        let (taken, _) = offered(&|d| d["servers"][0]["base"] = "https://elsewhere.example".into()).unwrap();
        assert_eq!(taken.unwrap().servers, built().servers);
        // a provider this build knows is never lost: gone from the copy, or in a shape this build cannot
        // read, it stays as compiled in; named as retired, it goes
        let (taken, said) = offered(&|d| d["providers"].as_array_mut().unwrap().retain(|p| p["id"] != "groq")).unwrap();
        assert!(taken.unwrap().providers.iter().any(|p| p.id == "groq") && said.left_out.iter().any(|l| l.starts_with("groq:")));
        let (taken, _) = offered(&|d| row(d, "groq")["key"] = "a new shape of key".into()).unwrap();
        assert_eq!(taken.unwrap().providers.iter().find(|p| p.id == "groq"), built().providers.iter().find(|p| p.id == "groq"));
        let (taken, _) = offered(&|d| {
            d["providers"].as_array_mut().unwrap().retain(|p| p["id"] != "groq");
            d["retired"] = serde_json::json!(["groq"]);
        })
        .unwrap();
        assert!(!taken.unwrap().providers.iter().any(|p| p.id == "groq"));
        // nothing may be emptied, and names stay names
        assert!(offered(&|d| d["providers"] = serde_json::json!([])).is_err());
        assert!(offered(&|d| row(d, "groq")["id"] = "none".into()).is_err());
        assert!(offered(&|d| row(d, "groq")["offers"][0]["asks"][0]["field"] = "key".into()).is_err());
        // a model whose files would land outside its folder, or come from somewhere else, or change under
        // the same version
        assert!(offered(&|d| d["models"][0]["kind"] = "..".into()).is_err());
        assert!(offered(&|d| d["models"][0]["files"][0][0] = "../../elsewhere".into()).is_err());
        assert!(offered(&|d| d["models"][0]["base"] = "https://elsewhere.example/models/x".into()).is_err());
        assert!(offered(&|d| d["models"][0]["files"][0][1] = 7.into()).is_err());
        assert!(offered(&|d| {
            d["models"][0]["files"][0][1] = 7.into();
            d["models"][0]["version"] = "2".into();
        })
        .unwrap()
        .1
        .newer);
    }

    #[test]
    fn a_copy_that_is_old_or_has_run_out_is_not_taken() {
        // not newer: nothing happens. The same number with other rows: kept as it is, and worth a line.
        assert_eq!(offered(&|d| d["rev"] = built().rev.into()).unwrap().1, Taken::default());
        let (taken, said) = offered(&|d| {
            d["rev"] = built().rev.into();
            d["hash"] = "another".into();
        })
        .unwrap();
        assert!(taken.is_none() && said.note.is_some_and(|n| n.contains("same rev")));
        // run out, or never said when it does: refused, so a copy replaced long ago cannot be served again
        assert!(offered(&|d| d["expires"] = (now() - 1).into()).is_err());
        assert!(offered(&|d| {
            d.as_object_mut().unwrap().remove("expires");
        })
        .is_err());
    }

    #[test]
    fn a_later_copy_cannot_change_how_a_coding_tool_is_started() {
        let built = read(BUILT_IN).unwrap();
        let with = |edit: &dyn Fn(&mut Value)| {
            let mut doc: Value = serde_json::from_str(BUILT_IN).unwrap();
            edit(&mut doc);
            read(&doc.to_string()).and_then(|c| allowed(&c, &built))
        };
        // every tool this build knows can be started, each launcher starting its own tool
        assert!(built.tools.iter().all(|t| !t.launchers.is_empty() && t.launchers.iter().all(|l| l.harness == t.id && !l.name.is_empty())));
        // what it is called may change; the program may not, and a tool new to this build brings none
        assert!(with(&|d| d["tools"][0]["name"] = "Claude".into()).is_ok());
        assert!(with(&|d| d["tools"][1]["launchers"][0]["held"]["program"] = serde_json::json!(["sh", "-c", "anything"])).is_err());
        assert!(with(&|d| d["tools"][0]["launchers"][0]["argv"][0] = "other".into()).is_err());
        assert!(with(&|d| d["tools"].as_array_mut().unwrap().push(serde_json::json!({ "id": "new", "name": "New", "launchers": [{ "name": "new", "held": { "by": "acp", "program": ["x"] } }] }))).is_err());
        assert!(with(&|d| d["tools"].as_array_mut().unwrap().push(serde_json::json!({ "id": "new", "name": "New" }))).is_ok());
    }

    #[test]
    fn a_copy_not_signed_by_airband_is_refused() {
        let doc = built_in(now() + 3600).replace(&format!("\"rev\":{}", built().rev), "\"rev\":999999");
        let forged = crate::audio::base64_encode(&[7u8; 64]);
        assert!(accept(doc.as_bytes(), &forged).is_err());
        assert!(current().rev < 999999);
    }
}
