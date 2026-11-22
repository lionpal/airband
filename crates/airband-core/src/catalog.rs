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
use crate::providers::{Job, Offer, Provider};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

mod check;
mod form;
mod kept;
mod programs;
mod rows;
pub use check::*;
pub use form::*;
pub use kept::*;
pub use programs::*;
use rows::*;

/// Where the signed copy is kept.
pub const ADDRESS: &str = "https://airband.dev/catalog/catalog.json";

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
    pub providers: Vec<Arc<Provider>>,
    /// The coding tools airband works with.
    pub tools: Vec<Tool>,
    pub models: Vec<Arc<PhoneModel>>,
    /// Programs the tower fetches to run a model on his computer. Always the ones compiled in.
    pub servers: Vec<Server>,
    /// The kinds of thing an agent's tool can say it made (`links.rs`): what to call each. The tools that
    /// name them are this build's own, so these are always the ones compiled in.
    pub links: Vec<crate::links::LinkKind>,
    /// The shapes credentials come in (`redact.rs`), one row an issuer's shape: what is kept out of the chat's
    /// copy of what is said. Always the ones compiled in, so no later copy can take one away.
    pub secrets: Vec<crate::redact::Shape>,
    /// Terminals a message can be typed into (`catalog/terminals/`): the program, and the words that type a
    /// line into one of its panes and press Enter. Always the ones compiled in: they are programs run.
    pub terminals: Vec<Terminal>,
    /// A program that pulls frames out of a video (`catalog/frames/`). Always the ones compiled in: they are
    /// programs run.
    pub frames: Vec<Frames>,
    /// The package managers airband is installed with (`catalog/installers/`). Always the ones compiled in:
    /// they are programs run.
    pub installers: Vec<Installer>,
    /// Words of the trade that a recogniser otherwise hears as everyday ones ("sync" as "sink").
    pub phrases: Vec<String>,
    /// Providers and coding tools that are no more, by id. One this build has compiled in and a later copy
    /// neither lists nor names here is kept as compiled in.
    pub retired: Vec<String>,
    /// What was in the document and could not be read by this build, in a few words each.
    pub left_out: Vec<String>,
}

/// A coding tool: what its agents are known by (`Agent::harness`), and what to call it where he picks the
/// tools he uses.
#[derive(Clone, PartialEq, Debug)]
pub struct Tool {
    pub id: String,
    pub name: String,
    pub maker: String,
    /// What he has to do once before this tool's agents are heard, when there is anything; shown where he
    /// picks the tool and where the app waits for its first agent.
    pub first: String,
    /// How the tower starts its agents, the one to prefer first. Each names a program that is run, so these
    /// are always the ones this build came with: a copy fetched later can change what a tool is called, and
    /// never what is run for it.
    pub launchers: Vec<crate::tools::Launcher>,
    /// What the code that speaks to this tool is told about it: the program's name, the addresses its
    /// accounts are asked at. Programs and addresses again, so these too are always this build's own.
    pub with: BTreeMap<String, Value>,
}

impl Tool {
    /// Something its row tells the code that speaks to it, as text. Empty when the row does not say.
    pub fn told(&self, what: &str) -> &str {
        row_text(self.with.get(what))
    }
}

/// Something a row tells the code, as text: what is written under one name in its `with`. Empty when the row
/// does not say, or says something other than text. For a word a row names (a program, an address, a field).
pub(crate) fn row_text(said: Option<&Value>) -> &str {
    said.and_then(Value::as_str).unwrap_or_default()
}

/// The same, for a value that is used as text whatever it is written as: a number or a yes/no as written,
/// words as [`row_text`] reads them. For a value told to an engine or a service.
pub(crate) fn scalar_text(said: Option<&Value>) -> String {
    match said {
        Some(v @ (Value::Number(_) | Value::Bool(_))) => v.to_string(),
        _ => row_text(said).to_string(),
    }
}

/// A model that runs on the phone itself.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct PhoneModel {
    pub kind: String,
    /// Its maker and its name.
    pub name: String,
    pub job: Job,
    /// The program that runs it, by name: one in the app's core or one its host has. A model is offered only
    /// where its engine is.
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
    /// For a voice model: the name of the voice used until he picks one. One of `voices`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub usual_voice: String,
    /// Who made it and under what licence, for the screen that offers it.
    #[serde(default)]
    pub credit: String,
    /// What its engine is told about this model, so that another model for the same engine is a row and no
    /// code: for a voice, the `rate` it speaks at, its `lang`, the `quant` its numbers are in and the `device`
    /// it runs on. And two things for any model. `prepare`: when the device has work of its own to do before
    /// the model first runs, the sentence to show while it does. `min_os`: where it runs only on some
    /// systems, each by name with the earliest major version (`{"ios": 18}`); a system not named cannot run
    /// it.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub with: std::collections::BTreeMap<String, Value>,
}

impl PhoneModel {
    /// The voice he named, or the one the row names as its usual one when he has named none.
    pub fn voice<'a>(&'a self, named: &'a str) -> &'a str {
        if named.is_empty() { &self.usual_voice } else { named }
    }
    /// One of the things its engine is told, as text. Empty when the row does not say.
    pub fn told(&self, name: &str) -> String {
        scalar_text(self.with.get(name))
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
    /// What it is started with. `{model}` is the model to run, `{port}` where to listen.
    #[serde(default)]
    pub args: Vec<String>,
    /// What is set in its environment. `{models}` is the folder its models are kept in.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
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

/// The catalogue in use. One is held at a time and shared by whoever is reading it; taking a newer copy
/// puts that one here, and the one before is dropped when its last reader lets go.
static CURRENT: RwLock<Option<Arc<Catalog>>> = RwLock::new(None);

fn built_shared() -> &'static Arc<Catalog> {
    static BUILT: std::sync::OnceLock<Arc<Catalog>> = std::sync::OnceLock::new();
    BUILT.get_or_init(|| Arc::new(read(BUILT_IN).expect("the catalogue built into this program does not read")))
}

/// The catalogue compiled into this program.
pub fn built() -> &'static Catalog {
    built_shared()
}

/// The catalogue in use: the newest one accepted, or the one built in.
pub fn current() -> Arc<Catalog> {
    if let Some(c) = CURRENT.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return c.clone();
    }
    CURRENT.write().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(|| built_shared().clone()).clone()
}

/// The document built into this program, as it would be published. `expires` is when a copy of it stops
/// being taken, in seconds since 1970.
pub fn built_in(expires: u64) -> String {
    let mut doc: Value = serde_json::from_str(BUILT_IN).expect("the catalogue built into this program does not read");
    doc["expires"] = expires.into();
    doc.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frames_program_is_this_builds_own_row_and_fills_its_words() {
        let f = frames().expect("a frames row is compiled in");
        let run = f.run("/v/clip.mov", 30, "/v/frames");
        assert!(run.contains(&"/v/clip.mov".to_string()) && run.contains(&"30".to_string()) && run.contains(&"/v/frames/%02d.jpg".to_string()), "{run:?}");
        assert!(!run.iter().any(|w| w.contains('{')), "{run:?}");
    }

    #[test]
    fn a_copy_whose_voice_row_names_no_usual_voice_of_its_own_is_not_well_formed() {
        check(built()).unwrap();
        let mut c = read(BUILT_IN).unwrap();
        let (p, o) = c.providers.iter().enumerate().find_map(|(i, p)| p.offers.iter().position(|o| !o.voices.is_empty()).map(|j| (i, j))).unwrap();
        let mut provider = (*c.providers[p]).clone();
        let mut offer = (*provider.offers[o]).clone();
        offer.asks.retain(|a| a.field != "voice");
        provider.offers[o] = Arc::new(offer);
        c.providers[p] = Arc::new(provider);
        assert!(check(&c).unwrap_err().contains("usual voice"));
        let mut c = read(BUILT_IN).unwrap();
        let m = c.models.iter().position(|m| m.job == Job::Voice).unwrap();
        let mut model = (*c.models[m]).clone();
        model.usual_voice = "nobody".into();
        c.models[m] = Arc::new(model);
        assert!(check(&c).unwrap_err().contains("usual voice"));
    }

    /// The built-in document with a change made to it, weighed as a copy offered to this build.
    fn offered(edit: &dyn Fn(&mut Value)) -> Result<(Option<Catalog>, Taken), String> {
        let mut doc: Value = serde_json::from_str(BUILT_IN).unwrap();
        doc["rev"] = (built().rev + 1).into();
        doc["expires"] = (crate::clock::now_secs() + 3600).into();
        edit(&mut doc);
        weigh(&doc.to_string(), crate::clock::now_secs(), built())
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
            r#"{"rev":9,"providers":[
            {"id":"x","name":"X","offers":[{"id":"a","job":"voice","by":{"interface":"something-new"},"label":"New"},{"id":"b","job":"voice","by":"speech","label":"No interface named"},{"id":"c","job":"ears","by":{"interface":"transcribe"},"label":"Known","preferred":"yes"},{"job":"ears","by":{"interface":"transcribe"},"label":"No id"}]},
            {"id":"y","name":"Y","key":"a new shape of key","offers":[]},
            {"id":"z","name":"Z","offers":[{"id":"r","job":"summary","by":{"interface":"chat"},"label":"Reads"}]}],
            "models":[{"kind":"m","name":"M"}]}"#,
        )
        .unwrap();
        // an interface nobody has code for still reads: whoever lists or calls leaves it out
        assert_eq!(c.providers.iter().map(|p| (p.id.as_str(), p.offers.iter().map(|o| o.label.as_str()).collect::<Vec<_>>())).collect::<Vec<_>>(), [("x", vec!["New"]), ("z", vec!["Reads"])]);
        // three offers and one provider and one model did not read, and each is named
        assert_eq!(c.left_out.len(), 5, "{:?}", c.left_out);
        assert!(c.left_out[0].starts_with("x: an offer") && c.left_out[3].starts_with("y:") && c.left_out[4].starts_with("m:"));
    }

    #[test]
    fn a_tool_row_this_build_cannot_read_is_left_out_said_and_the_compiled_in_tool_stays() {
        // N20: one tool row in a shape this build cannot read leaves the rest of the copy to be taken
        let (taken, said) = offered(&|d| {
            let tools = d["tools"].as_array_mut().unwrap();
            tools.iter_mut().find(|t| t["id"] == "codex").unwrap()["name"] = serde_json::json!({ "a": "new shape" });
            tools.push(serde_json::json!({ "id": "newtool", "launchers": "a new shape" }));
        })
        .unwrap();
        let taken = taken.unwrap();
        assert_eq!(taken.tools.iter().find(|t| t.id == "codex"), built().tools.iter().find(|t| t.id == "codex"));
        assert!(!taken.tools.iter().any(|t| t.id == "newtool"));
        assert!(said.left_out.iter().any(|l| l.starts_with("codex:")) && said.left_out.iter().any(|l| l.starts_with("newtool:")), "{:?}", said.left_out);
    }

    #[test]
    fn a_launcher_this_build_cannot_read_is_left_out_and_said_and_its_tool_and_other_launchers_stay() {
        let (taken, said) = offered(&|d| {
            let tools = d["tools"].as_array_mut().unwrap();
            let codex = tools.iter_mut().find(|t| t["id"] == "codex").unwrap();
            codex["launchers"].as_array_mut().unwrap().push(serde_json::json!({ "name": "later", "held": { "by": "a way this build has no code for" } }));
        })
        .unwrap();
        let taken = taken.unwrap();
        let codex = taken.tools.iter().find(|t| t.id == "codex").unwrap();
        let built_codex = built().tools.iter().find(|t| t.id == "codex").unwrap().clone();
        assert_eq!(codex.launchers, built_codex.launchers);
        assert!(said.left_out.iter().any(|l| l.starts_with("codex: a launcher")), "{:?}", said.left_out);
    }

    #[test]
    fn a_copy_without_an_end_is_refused_as_such() {
        // a copy that does not say when it ends is told as that, never as one that has run out
        let said = offered(&|d| {
            d.as_object_mut().unwrap().remove("expires");
        })
        .err()
        .unwrap();
        assert!(said.contains("does not say until when"), "{said}");
    }

    #[test]
    fn two_copies_taken_at_once_always_leave_the_newer_in_use() {
        let doc = |up: u64| {
            let mut doc: Value = serde_json::from_str(BUILT_IN).unwrap();
            doc["rev"] = (built().rev + up).into();
            doc["expires"] = (crate::clock::now_secs() + 3600).into();
            doc.to_string()
        };
        let (one, two) = (doc(1), doc(2));
        for _ in 0..50 {
            let slot: &'static RwLock<Option<Arc<Catalog>>> = Box::leak(Box::new(RwLock::new(None)));
            let (a, b) = (one.clone(), two.clone());
            let older = std::thread::spawn(move || check::take(slot, &a, crate::clock::now_secs()));
            let newer = std::thread::spawn(move || check::take(slot, &b, crate::clock::now_secs()));
            older.join().unwrap().unwrap();
            newer.join().unwrap().unwrap();
            assert_eq!(slot.read().unwrap().as_ref().unwrap().rev, built().rev + 2);
        }
    }

    #[test]
    fn a_server_link_terminal_frames_or_secret_row_that_cannot_be_used_is_left_out_and_said() {
        let c = read(
            r#"{"rev":9,"providers":[],"models":[],
            "servers":[{"id":"s1"}],
            "links":[{"id":"l1"}],
            "terminals":[{"id":"t1","program":"x"}],
            "frames":[{"id":"f1"}],
            "secrets":[{"id":"k1","prefix":"sk-"},{"id":"k2","prefix":"","least":8},{"id":"k3","prefix":"ab-","least":0}]}"#,
        )
        .unwrap();
        let named: Vec<&str> = c.left_out.iter().map(|l| l.split(':').next().unwrap_or("")).collect();
        assert_eq!(named, ["s1", "l1", "t1", "f1", "k1", "k2", "k3"], "{:?}", c.left_out);
        assert!(c.secrets.is_empty() && c.servers.is_empty() && c.terminals.is_empty());
    }

    #[test]
    fn a_later_copy_cannot_change_what_is_run_or_where_a_key_goes() {
        // as it is: taken. A new price, a new voice, a new model id, a new provider on a known interface: taken.
        assert!(offered(&|_| {}).unwrap().1.newer);
        assert!(offered(&|d| row(d, "gemini")["offers"][0]["price"] = "About $1".into()).unwrap().1.newer);
        assert!(offered(&|d| row(d, "openai")["offers"][0]["asks"][1]["usual"] = "gpt-7".into()).unwrap().1.newer);
        assert!(offered(&|d| d["providers"].as_array_mut().unwrap().push(serde_json::json!({ "id": "new", "name": "New", "key": { "name": "New key" }, "offers": [{ "id": "new", "job": "summary", "by": { "interface": "chat" }, "label": "New", "asks": [{ "field": "url", "question": "Address", "usual": "https://new.example/v1", "reach": true }] }] }))).unwrap().1.newer);
        // the program the tower runs for summaries
        assert!(offered(&|d| row(d, "claude")["offers"][0]["by"]["with"]["argv"] = serde_json::json!(["sh", "-c", "anything"])).is_err());
        // where a key he has entered is sent: by a setting, or by what the interface is told
        assert!(offered(&|d| row(d, "openai")["offers"][0]["asks"][0]["usual"] = "https://elsewhere.example/v1".into()).is_err());
        assert!(offered(&|d| row(d, "openai")["offers"][0]["asks"][0]["reach"] = false.into()).is_err());
        assert!(offered(&|d| row(d, "elevenlabs")["offers"][0]["by"]["with"]["url"] = "https://elsewhere.example".into()).is_err());
        // how a known service is called (the old call is not made in its place)
        assert!(offered(&|d| row(d, "gemini")["offers"][0]["by"]["with"]["shaped"] = false.into()).is_err());
        // an offer new to a known provider that would run a program, or be reached somewhere else, is left
        // out and said so; the rest of the copy is taken
        for odd in [serde_json::json!({ "id": "runs", "job": "voice", "by": { "interface": "command", "with": { "argv": ["x"] } }, "label": "Runs" }), serde_json::json!({ "id": "moved", "job": "voice", "by": { "interface": "speech" }, "label": "Moved", "asks": [{ "field": "url", "question": "Address", "usual": "https://elsewhere.example/v1", "reach": true }] })] {
            let (taken, said) = offered(&|d| row(d, "openrouter")["offers"].as_array_mut().unwrap().push(odd.clone())).unwrap();
            assert_eq!(taken.unwrap().providers.iter().find(|p| p.id == "openrouter"), built().providers.iter().find(|p| p.id == "openrouter"));
            assert!(said.left_out.iter().any(|l| l.starts_with("openrouter:")));
        }
        // a second offer that runs the very program a compiled-in one does is still not taken from a copy
        let (taken, said) = offered(&|d| {
            let offers = row(d, "claude")["offers"].as_array_mut().unwrap();
            let mut again = offers[0].clone();
            again["id"] = "again".into();
            offers.push(again);
        })
        .unwrap();
        assert!(taken.unwrap().providers.iter().find(|p| p.id == "claude").unwrap().offers.len() == 1 && said.left_out.len() == 1);
        // what an offer this build knows is for does not change
        assert!(offered(&|d| row(d, "openai")["offers"][0]["job"] = "voice".into()).is_err());
        // how a key is presented is part of how a service is reached
        assert!(offered(&|d| row(d, "openai")["offers"][0]["by"]["with"]["auth"]["header"] = "x-other".into()).is_err());
        // nor is where and how a key he gives is sent to be tested
        assert!(offered(&|d| row(d, "openai")["key"]["check"]["url"] = "https://elsewhere.example/models".into()).is_err());
        assert!(offered(&|d| row(d, "gemini")["key"]["check"]["auth"]["header"] = "x-other".into()).is_err());
        assert!(offered(&|d| row(d, "elevenlabs")["key"]["check"]["wrong_key"] = serde_json::json!([])).is_err());
        // a provider nobody has seen may not run a program: that offer is left out, and said so
        let (taken, said) = offered(&|d| d["providers"].as_array_mut().unwrap().push(serde_json::json!({ "id": "new", "name": "New", "offers": [{ "id": "new", "job": "summary", "by": { "interface": "command", "with": { "argv": ["x"] } }, "label": "New" }] }))).unwrap();
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
        assert!(offered(&|d| row(d, "groq")["offers"][0]["asks"][0]["field"] = "offer".into()).is_err());
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
    fn a_provider_can_do_one_job_more_than_one_way_under_one_key() {
        // the built-in voice offer of a provider, again under another id with another model: a second way to
        // do the same job, reached the same way
        let second = |d: &mut Value| {
            let offers = row(d, "openai")["offers"].as_array_mut().unwrap();
            let mut more = offers.iter().find(|o| o["job"] == "voice").unwrap().clone();
            more["id"] = "big-tts".into();
            more["label"] = "OpenAI Big TTS".into();
            more["asks"].as_array_mut().unwrap().iter_mut().find(|a| a["field"] == "model").unwrap()["usual"] = "big-tts-1".into();
            offers.push(more);
        };
        let (taken, said) = offered(&second).unwrap();
        let taken = taken.unwrap();
        let openai = taken.providers.iter().find(|p| p.id == "openai").unwrap();
        let voices: Vec<&str> = openai.offers.iter().filter(|o| o.job == Job::Voice).map(|o| o.id.as_str()).collect();
        assert_eq!(voices, ["mini-tts", "big-tts"]);
        assert!(said.left_out.is_empty() && openai.key);
        // each is used by its own id, and what one is set to says nothing of the other
        let model = |offer: &str| openai.offers.iter().find(|o| o.id == offer).unwrap().asks.iter().find(|a| a.field == "model").unwrap().usual.clone();
        assert_eq!((model("mini-tts"), model("big-tts")), ("gpt-4o-mini-tts".to_string(), "big-tts-1".to_string()));
        // two offers may not share an id
        assert!(offered(&|d| {
            second(d);
            let offers = row(d, "openai")["offers"].as_array_mut().unwrap();
            offers.last_mut().unwrap()["id"] = "mini-tts".into();
        })
        .is_err());
        // and an offer's id is a plain name that is not one of the words a setting is written with
        assert!(offered(&|d| row(d, "groq")["offers"][0]["id"] = "Not Plain".into()).is_err());
        assert!(offered(&|d| row(d, "groq")["offers"][0]["id"] = "key".into()).is_err());
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
        assert!(offered(&|d| d["expires"] = (crate::clock::now_secs() - 1).into()).is_err());
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
        // what a tool's code is told (a program's name, an address) is this build's own too
        assert!(with(&|d| d["tools"][0]["with"]["program"] = "other".into()).is_err());
        assert!(with(&|d| d["tools"][0]["with"]["accounts_token"] = "https://elsewhere.example/token".into()).is_err());
        // every tool this build knows can be started, each launcher starting its own tool
        assert!(built.tools.iter().all(|t| !t.launchers.is_empty() && t.launchers.iter().all(|l| l.harness == t.id && !l.name.is_empty())));
        // and every keyed offer says its own id and no two of a provider's share one
        check(&built).unwrap();
        // what it is called may change; the program may not, and a tool new to this build brings none
        assert!(with(&|d| d["tools"][0]["name"] = "Claude".into()).is_ok());
        assert!(with(&|d| d["tools"][1]["launchers"][0]["held"]["program"] = serde_json::json!(["sh", "-c", "anything"])).is_err());
        assert!(with(&|d| d["tools"][0]["launchers"][0]["argv"][0] = "other".into()).is_err());
        assert!(with(&|d| d["tools"].as_array_mut().unwrap().push(serde_json::json!({ "id": "new", "name": "New", "launchers": [{ "name": "new", "held": { "by": "acp", "program": ["x"] } }] }))).is_err());
        assert!(with(&|d| d["tools"].as_array_mut().unwrap().push(serde_json::json!({ "id": "new", "name": "New" }))).is_ok());
    }

    #[test]
    fn a_copy_not_signed_by_airband_is_refused() {
        let doc = built_in(crate::clock::now_secs() + 3600).replace(&format!("\"rev\":{}", built().rev), "\"rev\":999999");
        let forged = crate::encode::base64(&[7u8; 64]);
        assert!(accept(doc.as_bytes(), &forged).is_err());
        assert!(current().rev < 999999);
    }

    #[test]
    fn what_a_copy_holds_that_this_build_cannot_use_is_said() {
        let taken = Taken { newer: true, left_out: vec!["x: an offer (missing field `label`)".into()], note: None };
        // a build that speaks nothing: every offer in hand is for another build
        let lines = news(&taken, &|_| false);
        assert!(lines.len() == 2 && lines[0].contains("left out 1 rows") && lines[1].contains("for a newer build"), "{lines:?}");
        // one that speaks everything, and a copy with nothing odd in it: nothing to say
        assert!(news(&Taken { newer: true, ..Default::default() }, &|_| true).is_empty());
        // a copy that was not taken says only what is worth knowing about that
        let same = Taken { note: Some("a catalogue with the same rev (5) has different rows; the one in hand is kept".into()), ..Default::default() };
        assert_eq!(news(&same, &|_| false).len(), 1);
    }


    // N20: rows are read one at a time, tool rows too: one this build cannot read is left out and said, and the
    // rest of the copy is kept.
    #[test]
    fn n20_one_tool_row_this_build_cannot_read_leaves_out_only_that_row() {
        let mut doc: Value = serde_json::from_str(BUILT_IN).unwrap();
        let tools = doc["tools"].as_array().unwrap().len();
        assert!(tools > 0);
        doc["tools"].as_array_mut().unwrap().push(serde_json::json!({ "id": "from-a-later-build", "name": 7 }));
        let read = read(&doc.to_string());
        let c = read.as_ref().expect("one unreadable tool row refused the whole catalogue");
        assert_eq!(c.tools.len(), tools);
        assert!(c.left_out.iter().any(|l| l.contains("from-a-later-build")), "{:?}", c.left_out);
    }
}
