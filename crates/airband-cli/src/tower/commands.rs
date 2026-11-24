//! The commands and skills each agent's tool takes, for the phone to offer when he starts a message with `/`.
//! Which ones there are, and whether one runs from here, is the agent's adapter's to say (`adapters::commands`):
//! the tool asked, or what the tool listed for the session read with the files it was written in. The answer is
//! kept, and asked again only on a fact: a file or folder it was read from changed (by its time on the disk), the
//! tool listed anew, or the agent moved folder or way in. Nothing is asked again on a timer.
use super::*;
use airband_core::commands::{Command, Commands, Listed};

/// What was last found for each agent, by its id.
#[derive(Default)]
pub(super) struct Kept(tokio::sync::Mutex<HashMap<String, Found>>);

/// What one answer was read from: the agent as it was, what its tool had listed, and every file and folder read,
/// each with its time on the disk then.
#[derive(PartialEq)]
struct Read {
    cwd: Option<String>,
    deliver: Option<Deliver>,
    listed: Option<Vec<Listed>>,
    files: Vec<(std::path::PathBuf, Option<std::time::SystemTime>)>,
}
struct Found {
    from: Read,
    items: Vec<Command>,
}

/// Each file's time on the disk now; nothing for one that is not there.
fn stamped(files: impl IntoIterator<Item = std::path::PathBuf>) -> Vec<(std::path::PathBuf, Option<std::time::SystemTime>)> {
    files.into_iter().map(|f| {
        let at = crate::config::stamp_of(&f);
        (f, at)
    }).collect()
}

/// What this agent's tool takes now. Held while it is asked, so two asks at once start the tool once.
pub(super) async fn of(app: &Arc<App>, agent: &str) -> Result<Commands, String> {
    let (me, listed) = app.read().find(agent).map(|s| (s.agent.clone(), s.tool_commands.clone())).ok_or(NOT_RUNNING)?;
    let mut kept = app.commands.0.lock().await;
    if let Some(found) = kept.get(agent) {
        let now = Read { cwd: me.cwd.clone(), deliver: me.deliver.clone(), listed: listed.clone(), files: stamped(found.from.files.iter().map(|(f, _)| f.clone())) };
        if now == found.from {
            return Ok(Commands { items: found.items.clone() });
        }
    }
    let asked = me.clone();
    let told = listed.clone();
    let found = tokio::task::spawn_blocking(move || crate::adapters::commands(&asked, told.as_deref())).await.map_err(|_| "Its list could not be read just now.".to_string())?;
    let Some(found) = found else {
        kept.remove(agent);
        return Ok(Commands::default());
    };
    // the times are taken after the reading: a file changed while it was read is read again next time
    let from = Read { cwd: me.cwd, deliver: me.deliver, listed, files: stamped(found.watch) };
    log(format!("{}: {} commands and skills listed", short_id(agent), found.items.len()));
    kept.insert(agent.to_string(), Found { from, items: found.items.clone() });
    Ok(Commands { items: found.items })
}
