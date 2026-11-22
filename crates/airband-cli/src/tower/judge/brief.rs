//! The folder a command-line reader is run in, with its brief, and what readers are offered.
use super::*;

/// What a coding tool is told when it is used only to rewrite a message: one line, and no tools. Kept in a
/// folder of its own: as a plain file a tool can be pointed at, and in whatever other form a tool's row says
/// it looks for one (`brief` in the row). A general coding agent brings thousands of words of its
/// own instructions to every call; this is what keeps a summary small and quick.
pub(super) const BRIEF: &str = "You rewrite messages. Reply with the JSON asked for and nothing else.";

pub(super) fn brief_folder(by: &By) -> Result<std::path::PathBuf, String> {
    let folder = config::dir().join("brief");
    let mut files = vec![(folder.join("brief.md"), format!("{BRIEF}\n"))];
    // a tool that takes its brief as a file of its own kind, somewhere of its own inside the folder it runs
    // in: its row says where (`brief.path`) and what stands before the brief there (`brief.head`)
    if let Some(own) = by.with.get("brief") {
        let path = own["path"].as_str().map(std::path::Path::new).filter(|p| p.is_relative() && !p.components().any(|c| matches!(c, std::path::Component::ParentDir))).ok_or("the brief's place in this row is not inside the folder it runs in")?;
        files.push((folder.join(path), format!("{}{BRIEF}\n", own["head"].as_str().unwrap_or_default())));
    }
    if files.iter().any(|(path, text)| std::fs::read_to_string(path).ok().as_deref() != Some(text.as_str())) {
        for (path, text) in &files {
            std::fs::create_dir_all(path.parent().unwrap_or(&folder)).map_err(|e| e.to_string())?;
            std::fs::write(path, text).map_err(|e| e.to_string())?;
        }
    }
    Ok(folder)
}

/// What can read on this tower, for a radio that offers the choice: the tower's own list for its own job, so
/// a phone with another copy of the catalogue offers nothing this tower would refuse.
pub(in crate::tower) fn offered(cfg: &Config) -> airband_core::radio::ReadsOffered {
    use airband_core::radio::{ReadsAsk, ReadsOffer, ReadsOffered};
    let now = reader(cfg);
    let kept: std::collections::HashSet<String> = cfg.keys.values().map(|k| k.provider.clone()).collect();
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

pub(super) fn tail(text: &str, max_chars: usize) -> String {
    let n = text.chars().count();
    text.chars().skip(n.saturating_sub(max_chars)).collect()
}
