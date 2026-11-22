//! Commands for trying the tower's judgement by hand, and for `airband setup` to see that its model answers.
use super::*;

pub(crate) fn triage(cfg: Config) -> Result<(), String> {
    let mut full = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut full).map_err(|e| e.to_string())?;
    let rt = runtime()?;
    let item = Item { id: String::new(), notice: false, full: full.trim().to_string(), kind: None, say: None, asked: None };
    let j = rt.block_on(prepare(&Settings::held(cfg), &item));
    println!("{}\t{}\t{}", super::store::word(&j.kind).unwrap_or_default(), j.say, j.options.iter().enumerate().map(|(i, o)| if j.recommended == Some(i as u32) { format!("[{o}]") } else { o.clone() }).collect::<Vec<_>>().join(" | "));
    Ok(())
}

/// Does the model that reads for the tower answer? For `airband setup`.
pub(crate) fn try_model(cfg: &Config) -> Result<String, String> {
    let rt = runtime()?;
    // a model on this computer is fetched the first time: wait through that, and say so
    local::listen_after(cfg.port);
    if let Some((model, server)) = judge::local_model(&cfg.summarizer) {
        println!("  Fetching the model and starting it. The first time this is a few gigabytes and takes a while.");
        if let Err(why) = local::ensure(&model, &server, local::Patience::Long) {
            local::stop();
            return Err(why);
        }
    }
    println!("  Asking it something…");
    let answered = rt.block_on(judge::ask_model(&Settings::held(cfg.clone()), judge::READY.into())).map(|_| "It answers.".into());
    // a model this command started on this computer is stopped again: the tower starts its own
    local::stop();
    answered
}

/// `airband progress`: the labels of an agent's steps on standard input, one to a line; prints the note he
/// would be given for them, or "none". For trying the judgement by hand.
pub(crate) fn progress(cfg: Config) -> Result<(), String> {
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).map_err(|e| e.to_string())?;
    let steps: Vec<String> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
    let rt = runtime()?;
    println!("{}", rt.block_on(async { let s = Settings::held(cfg); let name = s.current().name.clone(); judge::progress(&s, &name, &steps).await }).unwrap_or_else(|| "none".into()));
    Ok(())
}

/// The runtime each of these commands runs its one question on.
fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())
}
