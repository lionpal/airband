//! The catalogue: every service and every on-device model airband can use, as data.
//!
//! Each is a small JSON file under `catalog/` at the top of the repository: one per provider
//! (`catalog/providers/<id>.json`) and one per model that runs on a phone (`catalog/models/<kind>.json`).
//! Building this crate puts them together into one document, which is compiled into the tower and into the
//! app, so both know the whole catalogue with no network at all.
//!
//! A newer copy of the same document can be taken later ([`accept`]): a new voice, a new model id or a new
//! price then shows without a new build. A copy is taken only when it is signed by airband's catalogue key,
//! whose public half is below, and only when its `rev` is higher than the one in hand. The signature matters:
//! a row says where a key of his is sent.
//!
//! What a row cannot do is add a way of calling a service. Those are code ([`By`]), and a row names one. A
//! row that names one this build does not have is left out, so an old build reads a new catalogue and shows
//! what it can use.
use crate::providers::{Ask, By, Job, Offer, Provider, Voice};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::RwLock;

/// Where the signed copy is kept. The `1` is the version of this document's form.
pub const ADDRESS: &str = "https://airband.dev/catalog/1/catalog.json";

/// The public half of the key that signs the hosted copy.
const SIGNED_BY: [u8; 32] = include!("catalog.key.in");

/// What was put together from `catalog/` when this was built.
const BUILT_IN: &str = include_str!(concat!(env!("OUT_DIR"), "/catalog.json"));

pub struct Catalog {
    /// Goes up with every change that is published.
    pub rev: u64,
    pub providers: &'static [Provider],
    /// The coding tools airband works with.
    pub tools: &'static [Tool],
    pub models: &'static [PhoneModel],
}

/// A coding tool: what its agents are known by (`Agent::harness`), and what to call it where he picks the
/// tools he uses.
pub struct Tool {
    pub id: &'static str,
    pub name: &'static str,
    pub maker: &'static str,
}

/// A model that runs on the phone itself.
pub struct PhoneModel {
    pub kind: &'static str,
    /// Its maker and its name.
    pub name: &'static str,
    pub job: Job,
    /// The program in the app that runs it. A build offers only the models it has the program for.
    pub engine: &'static str,
    /// Where its files are kept: each is fetched from here by its path.
    pub base: &'static str,
    /// Each file: its path under the model's folder, its size, and its SHA-256. A file that does not match is
    /// not kept.
    pub files: &'static [(&'static str, u64, &'static str)],
    /// For a voice model: the voices he can choose between, the usual one first. Each is a name (its file is
    /// `voices/<name>.safetensors`, and a short clip of it is at `previews/<name>.m4a` beside the model) and
    /// a few words on how it sounds.
    pub voices: &'static [(&'static str, &'static str)],
    /// Who made it and under what licence, for the screen that offers it.
    pub credit: &'static str,
}

static CURRENT: RwLock<Option<&'static Catalog>> = RwLock::new(None);

/// The catalogue in use: the newest one accepted, or the one built in.
pub fn current() -> &'static Catalog {
    if let Some(c) = *CURRENT.read().unwrap() {
        return c;
    }
    let built: &'static Catalog = Box::leak(Box::new(read(BUILT_IN).expect("the catalogue built into this program does not read")));
    *CURRENT.write().unwrap().get_or_insert(built)
}

/// The document built into this program, as it would be published.
pub fn built_in() -> &'static str {
    BUILT_IN
}

/// Take a newer copy of the catalogue. `signature` is the 64 bytes of its Ed25519 signature, in base64.
/// True when it is now the one in use; false when the one in hand is as new. An error when it is not
/// signed by airband's key or does not read, and then nothing changes.
pub fn accept(document: &[u8], signature: &str) -> Result<bool, String> {
    let key = VerifyingKey::from_bytes(&SIGNED_BY).map_err(|e| e.to_string())?;
    let sig = crate::audio::base64_decode(signature.trim()).ok_or("the catalogue's signature does not read")?;
    let sig = Signature::from_slice(&sig).map_err(|_| "the catalogue's signature does not read")?;
    key.verify(document, &sig).map_err(|_| "the catalogue is not signed by airband")?;
    let text = std::str::from_utf8(document).map_err(|e| e.to_string())?;
    let built: Catalog = read(BUILT_IN)?;
    // A provider this build has never heard of may not bring a program to run: such an offer is for a newer
    // build, and is left out here so the rest of the copy can still be taken.
    let mut doc: Value = serde_json::from_str(text).map_err(|e| format!("the catalogue does not read: {e}"))?;
    for p in doc["providers"].as_array_mut().into_iter().flatten() {
        let known = built.providers.iter().any(|b| Some(b.id) == p["id"].as_str());
        if let (false, Some(offers)) = (known, p["offers"].as_array_mut()) {
            offers.retain(|o| o["by"].get("command").is_none());
        }
    }
    let newer = read(&doc.to_string())?;
    if newer.rev <= current().rev {
        return Ok(false);
    }
    allowed(&newer, &built)?;
    *CURRENT.write().unwrap() = Some(Box::leak(Box::new(newer)));
    Ok(true)
}

/// What a copy taken later may not do, whoever signed it. The signature says it came from airband; these say
/// that even airband's own key cannot, through a catalogue, run a different program on his computer or send
/// a key he has already entered somewhere new. Those take a new build, which he installs himself.
///
/// - A provider this build knows keeps the command it runs and every address it is reached at.
/// - A provider this build does not know cannot be a command at all.
/// - Nothing may be emptied: a copy with no providers is refused.
/// - A model's name and its files' paths stay inside its own folder, and its files come from airband's address.
fn allowed(newer: &Catalog, built: &Catalog) -> Result<(), String> {
    if newer.providers.is_empty() {
        return Err("the catalogue has no providers in it".into());
    }
    let address = |o: &Offer| o.asks.iter().find(|a| a.field == "url").map(|a| a.usual);
    for p in newer.providers {
        let known = built.providers.iter().find(|b| b.id == p.id);
        for o in p.offers {
            let was = known.and_then(|b| b.offers.iter().find(|x| x.job == o.job));
            match (o.by, was.map(|w| w.by)) {
                (By::Command(now), Some(By::Command(then))) if now == then => {}
                (By::Command(_), _) => return Err(format!("the catalogue would change the program run for {}", p.id)),
                (By::OnThisComputer(_), _) | (_, None) => {}
                (_, Some(_)) => {
                    if address(o) != was.and_then(address) {
                        return Err(format!("the catalogue would change where {} is reached", p.id));
                    }
                }
            }
        }
        if known.is_some_and(|b| b.key != p.key) {
            return Err(format!("the catalogue would change whether {} takes a key", p.id));
        }
    }
    let plain = |name: &str| !name.is_empty() && !name.starts_with('.') && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_');
    for m in newer.models {
        if !plain(m.kind) || !m.base.starts_with("https://airband.dev/models/") {
            return Err(format!("the catalogue names a model that is not airband's to offer: {}", m.kind));
        }
        if m.files.iter().any(|(path, _, sha)| path.starts_with('/') || path.split('/').any(|part| !plain(part)) || sha.len() != 64) {
            return Err(format!("the catalogue gives {} a file outside its own folder", m.kind));
        }
    }
    Ok(())
}

// The document as written. Everything is optional that has a plain default, so a file says only what is so.

#[derive(Serialize, Deserialize)]
struct Document {
    rev: u64,
    providers: Vec<ProviderRow>,
    #[serde(default)]
    models: Vec<Value>,
    #[serde(default)]
    tools: Vec<ToolRow>,
}

fn yes() -> bool {
    true
}
fn no(b: &bool) -> bool {
    !*b
}
fn is_yes(b: &bool) -> bool {
    *b
}

#[derive(Serialize, Deserialize)]
struct ProviderRow {
    id: String,
    name: String,
    /// Present when it takes a key: what the key is called, and where keys are made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<KeyRow>,
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    offered: bool,
    /// Each is an [`OfferRow`]; one this build cannot read (a way of calling it does not have) is left out.
    offers: Vec<Value>,
}

#[derive(Serialize, Deserialize)]
struct ToolRow {
    id: String,
    name: String,
    #[serde(default)]
    maker: String,
}

#[derive(Serialize, Deserialize)]
struct KeyRow {
    name: String,
    #[serde(default)]
    at: String,
}

#[derive(Serialize, Deserialize)]
struct OfferRow {
    job: JobRow,
    by: ByRow,
    label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    about: String,
    #[serde(default, skip_serializing_if = "no")]
    preferred: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    price: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    login: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    voices: Vec<VoiceRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    asks: Vec<AskRow>,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum JobRow {
    Summary,
    Voice,
    Ears,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ByRow {
    Chat {
        #[serde(default)]
        cold: bool,
        #[serde(default)]
        shaped: bool,
    },
    OnThisComputer {
        model: String,
    },
    Command {
        #[serde(default)]
        argv: Vec<String>,
    },
    Speech,
    GeminiVoice,
    /// Written `{"azure_voice": {}}`: builds from before the relay stopped lending its speech service read a
    /// setting here, and a row written this way still reads in them.
    AzureVoice {},
    ElevenlabsVoice,
    Listener {
        dialect: String,
    },
    Transcribe,
}

#[derive(Serialize, Deserialize)]
struct VoiceRow {
    id: String,
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    about: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    sample: String,
}

#[derive(Serialize, Deserialize)]
struct AskRow {
    field: String,
    question: String,
    #[serde(default)]
    usual: String,
}

#[derive(Serialize, Deserialize)]
struct ModelRow {
    kind: String,
    name: String,
    job: JobRow,
    engine: String,
    base: String,
    files: Vec<(String, u64, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    voices: Vec<(String, String)>,
    #[serde(default)]
    credit: String,
}

/// Kept for as long as the program runs: a catalogue is read once at the start and perhaps once more later.
fn keep(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}
fn keep_all<T>(v: Vec<T>) -> &'static [T] {
    Box::leak(v.into_boxed_slice())
}

fn job(j: JobRow) -> Job {
    match j {
        JobRow::Summary => Job::Summary,
        JobRow::Voice => Job::Voice,
        JobRow::Ears => Job::Ears,
    }
}

fn read(text: &str) -> Result<Catalog, String> {
    let doc: Document = serde_json::from_str(text).map_err(|e| format!("the catalogue does not read: {e}"))?;
    let providers = doc
        .providers
        .into_iter()
        .map(|p| {
            let offers: Vec<Offer> = p
                .offers
                .into_iter()
                .filter_map(|o| serde_json::from_value::<OfferRow>(o).ok())
                .filter(|o| !matches!(&o.by, ByRow::Listener { dialect } if !crate::providers::DIALECTS.contains(&dialect.as_str())))
                .map(|o| Offer {
                    job: job(o.job),
                    by: match o.by {
                        ByRow::Chat { cold, shaped } => By::Chat { cold, shaped },
                        ByRow::OnThisComputer { model } => By::OnThisComputer(keep(model)),
                        ByRow::Command { argv } => By::Command(keep_all(argv.into_iter().map(keep).collect())),
                        ByRow::Speech => By::Speech,
                        ByRow::GeminiVoice => By::GeminiVoice,
                        ByRow::AzureVoice {} => By::AzureVoice,
                        ByRow::ElevenlabsVoice => By::ElevenlabsVoice,
                        ByRow::Listener { dialect } => By::Listener(keep(dialect)),
                        ByRow::Transcribe => By::Transcribe,
                    },
                    label: keep(o.label),
                    about: keep(o.about),
                    preferred: o.preferred,
                    price: keep(o.price),
                    login: keep(o.login),
                    voices: keep_all(o.voices.into_iter().map(|v| Voice { id: keep(v.id), name: keep(v.name), about: keep(v.about), sample: keep(v.sample) }).collect()),
                    asks: keep_all(o.asks.into_iter().map(|a| Ask { field: keep(a.field), question: keep(a.question), usual: keep(a.usual) }).collect()),
                })
                .collect();
            let (key, key_name, keys_at) = match p.key {
                Some(k) => (true, keep(k.name), keep(k.at)),
                None => (false, "", ""),
            };
            Provider { id: keep(p.id), name: keep(p.name), key, key_name, keys_at, offered: p.offered, offers: keep_all(offers) }
        })
        .collect();
    let models = doc
        .models
        .into_iter()
        .filter_map(|m| serde_json::from_value::<ModelRow>(m).ok())
        .map(|m| PhoneModel {
            kind: keep(m.kind),
            name: keep(m.name),
            job: job(m.job),
            engine: keep(m.engine),
            base: keep(m.base),
            files: keep_all(m.files.into_iter().map(|(path, size, sha)| (keep(path), size, keep(sha))).collect()),
            voices: keep_all(m.voices.into_iter().map(|(name, about)| (keep(name), keep(about))).collect()),
            credit: keep(m.credit),
        })
        .collect();
    let tools = doc.tools.into_iter().map(|t| Tool { id: keep(t.id), name: keep(t.name), maker: keep(t.maker) }).collect();
    Ok(Catalog { rev: doc.rev, providers: keep_all(providers), tools: keep_all(tools), models: keep_all(models) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_catalogue_reads_whole() {
        let doc: Document = serde_json::from_str(BUILT_IN).unwrap();
        // every offer and model in the repository's own files is one this build can use
        for p in &doc.providers {
            for o in &p.offers {
                serde_json::from_value::<OfferRow>(o.clone()).unwrap_or_else(|e| panic!("{}: {e}", p.id));
            }
        }
        for m in &doc.models {
            serde_json::from_value::<ModelRow>(m.clone()).unwrap();
        }
        assert!(current().rev >= 1 && !current().providers.is_empty() && !current().models.is_empty());
    }

    #[test]
    fn a_row_this_build_cannot_call_is_left_out_and_the_rest_is_kept() {
        let c = read(r#"{"rev":9,"providers":[{"id":"x","name":"X","offers":[
            {"job":"voice","by":{"something_new":{}},"label":"New"},
            {"job":"voice","by":"speech","label":"Known"}]}]}"#)
        .unwrap();
        assert_eq!(c.providers[0].offers.iter().map(|o| o.label).collect::<Vec<_>>(), ["Known"]);
        // the same for a listening dialect this build does not speak
        let c = read(r#"{"rev":9,"providers":[{"id":"x","name":"X","offers":[{"job":"ears","by":{"listener":{"dialect":"martian"}},"label":"New"}]}]}"#).unwrap();
        assert!(c.providers[0].offers.is_empty());
    }

    #[test]
    fn a_later_copy_cannot_change_what_is_run_or_where_a_key_goes() {
        let built = read(BUILT_IN).unwrap();
        let with = |edit: &dyn Fn(&mut Value)| {
            let mut doc: Value = serde_json::from_str(BUILT_IN).unwrap();
            edit(&mut doc);
            read(&doc.to_string()).and_then(|c| allowed(&c, &built))
        };
        let provider = |doc: &mut Value, id: &str| doc["providers"].as_array_mut().unwrap().iter_mut().find(|p| p["id"] == id).unwrap().clone();
        let _ = provider;
        // as it is: fine. A new price or voice: fine.
        assert!(with(&|_| {}).is_ok());
        assert!(with(&|d| d["providers"].as_array_mut().unwrap().iter_mut().find(|p| p["id"] == "gemini").unwrap()["offers"][0]["price"] = "About $1".into()).is_ok());
        // the program the tower runs for summaries
        assert!(with(&|d| d["providers"].as_array_mut().unwrap().iter_mut().find(|p| p["id"] == "claude").unwrap()["offers"][0]["by"]["command"]["argv"] = serde_json::json!(["sh", "-c", "anything"])).is_err());
        // where a key he has entered is sent
        assert!(with(&|d| d["providers"].as_array_mut().unwrap().iter_mut().find(|p| p["id"] == "openai").unwrap()["offers"][0]["asks"][0]["usual"] = "https://elsewhere.example/v1".into()).is_err());
        // a provider nobody has seen may not be a command (taking a copy leaves such an offer out), and the
        // list may not be emptied
        assert!(with(&|d| d["providers"].as_array_mut().unwrap().push(serde_json::json!({ "id": "new", "name": "New", "offers": [{ "job": "summary", "by": { "command": { "argv": ["x"] } }, "label": "New" }] }))).is_err());
        assert!(with(&|d| d["providers"] = serde_json::json!([])).is_err());
        // a model whose files would land outside its folder, or come from somewhere else
        assert!(with(&|d| d["models"][0]["kind"] = "..".into()).is_err());
        assert!(with(&|d| d["models"][0]["files"][0][0] = "../../elsewhere".into()).is_err());
        assert!(with(&|d| d["models"][0]["base"] = "https://elsewhere.example/models/x".into()).is_err());
    }

    #[test]
    fn a_copy_not_signed_by_airband_is_refused() {
        let doc = br#"{"rev":999999,"providers":[]}"#;
        let forged = crate::audio::base64_encode(&[7u8; 64]);
        assert!(accept(doc, &forged).is_err());
        assert!(current().rev < 999999);
    }
}
