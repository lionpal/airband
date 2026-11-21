//! The first form of the catalogue document, for the builds that read only that one: the apps up to
//! TestFlight build 83 and the towers up to 0.2.0 and the day after. They fetch
//! `https://airband.dev/catalog/1/catalog.json`, refuse a whole document they cannot read, and refuse one
//! that changes a command or an address they have compiled in.
//!
//! Nothing in this build reads form 1. It is only written here ([`export`], from the same rows as form 2)
//! and checked the way those builds would check it ([`check`]), so they keep getting new prices, voices and
//! model ids for what they already know. What is new since (an interface they have no code for, an offer
//! that needs something they cannot do, a new version of a model) is left out of it.
//!
//! When no build that reads form 1 is in use any more, this file and `catalog_v1_rev4.json` go, with the
//! `--form 1` half of `scripts/publish-catalog.sh`.
use serde::Deserialize;
use serde_json::{Value, json};

/// The catalogue as compiled into the last builds that read form 1 (rev 4). They hold a later copy to this.
const BASELINE: &str = include_str!("catalog_v1_rev4.json");

/// An offer's `by` as form 1 writes it. Nothing for an interface that form never had.
fn by(job: &str, now: &Value) -> Option<Value> {
    let with = &now["with"];
    Some(match (job, now["interface"].as_str()?) {
        ("summary", "chat") => json!({ "chat": { "cold": with["cold"].as_bool().unwrap_or(false), "shaped": with["shaped"].as_bool().unwrap_or(false) } }),
        ("summary", "on-this-computer") => json!({ "on_this_computer": { "model": with["model"] } }),
        ("summary", "command") => json!({ "command": { "argv": with["argv"] } }),
        ("voice", "speech") => json!("speech"),
        ("voice", "gemini") => json!("gemini_voice"),
        ("voice", "azure") => json!({ "azure_voice": {} }),
        ("voice", "elevenlabs") => json!("elevenlabs_voice"),
        ("ears", "transcribe") => json!("transcribe"),
        ("ears", dialect @ ("azure" | "gemini" | "elevenlabs")) => json!({ "listener": { "dialect": dialect } }),
        _ => return None,
    })
}

/// A form 2 document as form 1.
pub fn export(form_2: &str) -> Result<String, String> {
    let doc: Value = serde_json::from_str(form_2).map_err(|e| e.to_string())?;
    let baseline: Value = serde_json::from_str(BASELINE).map_err(|e| e.to_string())?;
    let known = |id: &Value, job: &Value| baseline["providers"].as_array().into_iter().flatten().find(|p| p["id"] == *id).map(|p| p["offers"].as_array().into_iter().flatten().any(|o| o["job"] == *job));
    let mut providers = Vec::new();
    for p in doc["providers"].as_array().into_iter().flatten() {
        let mut offers = Vec::new();
        for o in p["offers"].as_array().into_iter().flatten() {
            let Some(by) = o["job"].as_str().and_then(|job| by(job, &o["by"])) else { continue };
            let goes = match known(&p["id"], &o["job"]) {
                // what those builds have compiled in: they keep getting it
                Some(true) => true,
                // a job they never knew this provider to do, or a provider they never knew: only when it is
                // called exactly as they would call it, and runs nothing
                Some(false) | None => o["needs"].as_array().is_none_or(|n| n.is_empty()) && by.get("command").is_none() && by.get("on_this_computer").is_none(),
            };
            if !goes {
                continue;
            }
            let mut row = serde_json::Map::new();
            row.insert("job".into(), o["job"].clone());
            row.insert("by".into(), by);
            for field in ["label", "about", "preferred", "price", "login", "voices"] {
                if let Some(v) = o.get(field) {
                    row.insert(field.into(), v.clone());
                }
            }
            if let Some(asks) = o["asks"].as_array() {
                row.insert("asks".into(), asks.iter().map(|a| json!({ "field": a["field"], "question": a["question"], "usual": a["usual"].as_str().unwrap_or_default() })).collect());
            }
            offers.push(Value::Object(row));
        }
        if offers.is_empty() {
            continue;
        }
        let mut row = serde_json::Map::new();
        for field in ["id", "name", "key", "offered"] {
            if let Some(v) = p.get(field) {
                row.insert(field.into(), v.clone());
            }
        }
        row.insert("offers".into(), offers.into());
        providers.push(Value::Object(row));
    }
    // a version of a model, once published, never changes: those builds keep the versions they know
    let out = json!({ "rev": doc["rev"], "providers": providers, "models": baseline["models"], "tools": doc["tools"] });
    Ok(format!("{out}\n"))
}

// A form 1 document as the builds that read it do: the head of each provider strictly, each offer by itself.

#[derive(Deserialize)]
struct Document {
    rev: u64,
    providers: Vec<ProviderRow>,
    #[serde(default)]
    models: Vec<Value>,
}

#[derive(Deserialize)]
struct ProviderRow {
    id: String,
    #[allow(dead_code)]
    name: String,
    #[serde(default)]
    key: Option<KeyRow>,
    #[serde(default)]
    #[allow(dead_code)]
    offered: Option<bool>,
    offers: Vec<Value>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct KeyRow {
    name: String,
    #[serde(default)]
    at: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct OfferRow {
    job: JobRow,
    by: ByRow,
    label: String,
    #[serde(default)]
    about: String,
    #[serde(default)]
    preferred: bool,
    #[serde(default)]
    price: String,
    #[serde(default)]
    login: String,
    #[serde(default)]
    voices: Vec<VoiceRow>,
    #[serde(default)]
    asks: Vec<AskRow>,
}

#[derive(Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
enum JobRow {
    Summary,
    Voice,
    Ears,
}

#[derive(Deserialize, PartialEq)]
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
    AzureVoice {},
    ElevenlabsVoice,
    Listener {
        dialect: String,
    },
    Transcribe,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct VoiceRow {
    id: String,
    name: String,
    #[serde(default)]
    about: String,
    #[serde(default)]
    sample: String,
}

#[derive(Deserialize)]
struct AskRow {
    field: String,
    #[allow(dead_code)]
    question: String,
    #[serde(default)]
    usual: String,
}

fn read(text: &str) -> Result<(u64, Vec<(ProviderRow, Vec<OfferRow>)>, Vec<Value>), String> {
    let doc: Document = serde_json::from_str(text).map_err(|e| format!("a build that reads form 1 would refuse all of it: {e}"))?;
    let mut providers = Vec::new();
    for mut p in doc.providers {
        let mut offers = Vec::new();
        for o in std::mem::take(&mut p.offers) {
            // those builds would leave such an offer out without a word: here it is an error, so it is seen
            offers.push(serde_json::from_value::<OfferRow>(o).map_err(|e| format!("{}: an offer that builds reading form 1 would drop: {e}", p.id))?);
        }
        providers.push((p, offers));
    }
    Ok((doc.rev, providers, doc.models))
}

/// Would the builds that read form 1 take this document, all of it? The same tests they make against what
/// they have compiled in: every row reads, nothing they know is gone, no command and no address they know
/// has changed, no new provider brings a command, and the models are the ones they have.
pub fn check(form_1: &str) -> Result<(), String> {
    let (rev, providers, models) = read(form_1)?;
    let (was_rev, was, was_models) = read(BASELINE)?;
    if rev <= was_rev {
        return Err(format!("its rev is {rev}: builds that read form 1 have rev {was_rev} compiled in and take only a higher one"));
    }
    if providers.is_empty() {
        return Err("it has no providers in it".into());
    }
    if models != was_models {
        return Err("it changes a model those builds have, which they would fetch into the same folder".into());
    }
    let address = |o: &OfferRow| o.asks.iter().find(|a| a.field == "url").map(|a| a.usual.clone());
    for (old, old_offers) in &was {
        let Some((now, offers)) = providers.iter().find(|(p, _)| p.id == old.id) else { return Err(format!("{} is gone from it, and those builds may be using it", old.id)) };
        if now.key.is_some() != old.key.is_some() {
            return Err(format!("it changes whether {} takes a key", old.id));
        }
        for o in offers {
            let then = old_offers.iter().find(|x| x.job == o.job);
            match (&o.by, then.map(|t| &t.by)) {
                (ByRow::Command { argv }, Some(ByRow::Command { argv: before })) if argv == before => {}
                (ByRow::Command { .. }, _) => return Err(format!("it changes the program run for {}", old.id)),
                (ByRow::OnThisComputer { .. }, _) | (_, None) => {}
                (_, Some(_)) => {
                    if address(o) != then.and_then(address) {
                        return Err(format!("it changes where {} is reached", old.id));
                    }
                }
            }
        }
        if let Some(gone) = old_offers.iter().find(|x| !offers.iter().any(|o| o.job == x.job)) {
            let job = match gone.job {
                JobRow::Summary => "reads",
                JobRow::Voice => "speaks",
                JobRow::Ears => "listens",
            };
            return Err(format!("{} no longer {job} in it, and those builds may be using it for that", old.id));
        }
    }
    for (p, offers) in providers.iter().filter(|(p, _)| !was.iter().any(|(old, _)| old.id == p.id)) {
        if offers.iter().any(|o| matches!(o.by, ByRow::Command { .. })) {
            return Err(format!("{} is new and brings a command, which those builds refuse", p.id));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form_1() -> Value {
        serde_json::from_str(&export(&crate::catalog::built_in(u64::MAX)).unwrap()).unwrap()
    }

    #[test]
    fn todays_rows_as_form_1_are_what_the_older_builds_would_take() {
        let doc = form_1();
        check(&doc.to_string()).unwrap();
        // every provider those builds know is still there, reached as they reach it
        let baseline: Value = serde_json::from_str(BASELINE).unwrap();
        for old in baseline["providers"].as_array().unwrap() {
            let now = doc["providers"].as_array().unwrap().iter().find(|p| p["id"] == old["id"]).unwrap_or_else(|| panic!("{} is gone", old["id"]));
            for was in old["offers"].as_array().unwrap() {
                let o = now["offers"].as_array().unwrap().iter().find(|o| o["job"] == was["job"]).unwrap();
                assert_eq!(serde_json::from_value::<ByRow>(o["by"].clone()).unwrap() == serde_json::from_value::<ByRow>(was["by"].clone()).unwrap(), true, "{} {}", old["id"], was["job"]);
            }
        }
    }

    #[test]
    fn what_the_older_builds_would_refuse_is_caught_before_it_is_published() {
        let with = |edit: &dyn Fn(&mut Value)| {
            let mut doc = form_1();
            edit(&mut doc);
            check(&doc.to_string())
        };
        let row = |doc: &mut Value, id: &str| doc["providers"].as_array().unwrap().iter().position(|p| p["id"] == id).unwrap();
        assert!(with(&|_| {}).is_ok());
        // a new price reaches them
        assert!(with(&|d| {
            let at = row(d, "gemini");
            d["providers"][at]["offers"][0]["price"] = "About $1".into();
        })
        .is_ok());
        // a command, an address, a provider gone, a row of a shape they cannot read, a number not raised
        assert!(with(&|d| {
            let at = row(d, "codex");
            d["providers"][at]["offers"][0]["by"]["command"]["argv"][0] = "other".into();
        })
        .is_err());
        assert!(with(&|d| {
            let at = row(d, "openai");
            d["providers"][at]["offers"][0]["asks"][0]["usual"] = "https://elsewhere.example/v1".into();
        })
        .is_err());
        assert!(with(&|d| {
            let at = row(d, "groq");
            d["providers"].as_array_mut().unwrap().remove(at);
        })
        .is_err());
        assert!(with(&|d| {
            let at = row(d, "groq");
            d["providers"][at]["offers"][0]["by"] = json!({ "interface": "chat" });
        })
        .is_err());
        assert!(with(&|d| d["rev"] = 4.into()).is_err());
        assert!(with(&|d| d["models"][0]["files"][0][1] = 7.into()).is_err());
    }

    #[test]
    fn what_is_new_since_is_left_out_of_form_1() {
        let mut doc: Value = serde_json::from_str(&crate::catalog::built_in(u64::MAX)).unwrap();
        let providers = doc["providers"].as_array_mut().unwrap();
        // an interface form 1 never had; a new provider that needs something those builds cannot do; and a
        // new provider called just as they would call it
        providers.push(json!({ "id": "newvoice", "name": "N", "offers": [{ "job": "voice", "by": { "interface": "something-new" }, "label": "N" }] }));
        providers.push(json!({ "id": "newtool", "name": "N", "offers": [{ "job": "summary", "by": { "interface": "chat" }, "label": "N", "needs": ["auth-header"] }] }));
        providers.push(json!({ "id": "newchat", "name": "N", "key": { "name": "N key" }, "offers": [{ "job": "summary", "by": { "interface": "chat", "with": { "cold": true } }, "label": "N", "asks": [{ "field": "url", "question": "Address", "usual": "https://n.example/v1", "reach": true }] }] }));
        let out: Value = serde_json::from_str(&export(&doc.to_string()).unwrap()).unwrap();
        let ids: Vec<&str> = out["providers"].as_array().unwrap().iter().filter_map(|p| p["id"].as_str()).collect();
        assert!(ids.contains(&"newchat") && !ids.contains(&"newvoice") && !ids.contains(&"newtool"));
        check(&out.to_string()).unwrap();
    }
}
