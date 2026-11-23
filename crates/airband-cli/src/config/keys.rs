//! A key kept in the settings, as the file has it: each place that uses one names it, and the name is made
//! the key again when the settings are read.

/// The places in the settings that use a key.
pub(crate) const USERS: [&str; 1] = ["summarizer"];
/// What a use of a kept key is written as, before the key's name.
pub(crate) const BY_NAME: &str = "key:";

/// In the settings as they are written: each use of a kept key becomes the key's name.
pub(super) fn name_keys(doc: &mut serde_json::Value) {
    let kept: Vec<(String, String)> = doc["keys"].as_object().map(|m| m.iter().filter_map(|(n, k)| Some((n.clone(), k["secret"].as_str()?.to_string()))).collect()).unwrap_or_default();
    for user in USERS {
        let Some(key) = doc.get_mut(user).and_then(|u| u.get_mut("key")) else { continue };
        if let Some((name, _)) = kept.iter().find(|(_, secret)| !secret.is_empty() && key.as_str() == Some(secret.as_str())) {
            *key = serde_json::Value::String(format!("{BY_NAME}{name}"));
        }
    }
}

/// And as they are read: each name becomes the key again. Err names one that is not kept.
pub(super) fn fill_keys(doc: &mut serde_json::Value) -> Result<(), String> {
    for user in USERS {
        let Some(name) = doc.get(user).and_then(|u| u["key"].as_str()).and_then(|k| k.strip_prefix(BY_NAME)).map(str::to_string) else { continue };
        let secret = doc["keys"][&name]["secret"].as_str().ok_or_else(|| format!("the settings use a key called \"{name}\" that is not kept; run airband setup"))?.to_string();
        doc[user]["key"] = serde_json::Value::String(secret);
    }
    Ok(())
}
