//! Commands for trying the tower's judgement by hand, and for `airband setup` to see that its model answers.
use super::*;

pub fn triage(cfg: Config) -> Result<(), String> {
    let mut full = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut full).map_err(|e| e.to_string())?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let item = Item { id: String::new(), notice: false, full: full.trim().to_string(), kind: None, say: None, asked: None };
    let j = rt.block_on(prepare(&cfg, &item));
    println!("{}\t{}\t{}", serde_json::to_value(j.kind).unwrap().as_str().unwrap_or(""), j.say, j.options.iter().enumerate().map(|(i, o)| if j.recommended == Some(i as u32) { format!("[{o}]") } else { o.clone() }).collect::<Vec<_>>().join(" | "));
    Ok(())
}

/// Does the model that reads for the tower answer? For `airband setup`.
pub fn try_model(cfg: &Config) -> Result<String, String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    // a model on this computer is fetched the first time: wait through that, and say so
    local::listen_after(cfg.port);
    local::wait_for_it(true);
    if judge::local_model(&cfg.summarizer).is_some() {
        println!("  Fetching the model and starting it. The first time this is a few gigabytes and takes a while.");
    }
    println!("  Asking it something…");
    let answered = rt.block_on(judge::ask_model(cfg, "Reply with the one word: ready".into())).map(|_| "It answers.".into());
    // a model this command started on this computer is stopped again: the tower starts its own
    local::stop();
    answered
}

/// `airband progress`: the labels of an agent's steps on standard input, one to a line; prints the note he
/// would be given for them, or "none". For trying the judgement by hand.
pub fn progress(cfg: Config) -> Result<(), String> {
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).map_err(|e| e.to_string())?;
    let steps: Vec<String> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    println!("{}", rt.block_on(judge::progress(&cfg, &steps)).unwrap_or_else(|| "none".into()));
    Ok(())
}
