//! Words he said aloud to one agent, read for whether they ask its tool to run one of its commands or skills
//! ("slash compact", "use the review skill on main"). A model reads them against the list the agent's tool
//! gave, by number; what it answers is a number on that list and the words he gave it, and the agent is then
//! given the command as its tool takes it (`/name what he gave it`). Typed words are never read this way: he
//! types a command as the tool takes it.
#![expect(clippy::disallowed_macros, reason = "the model's answer shape")]
use super::*;

#[derive(Deserialize)]
struct Out {
    #[serde(default)]
    command: Option<usize>,
    #[serde(default)]
    arguments: String,
}

fn out_shape() -> Value {
    json!({
        "type": "object",
        "properties": { "command": { "type": ["integer", "null"] }, "arguments": { "type": "string" } },
        "required": ["command", "arguments"],
        "additionalProperties": false
    })
}

/// The command his words ask this agent's tool to run, as the tool takes it; nothing when they ask none, when
/// none of its commands runs from here, or when no model could read them.
pub(super) async fn read(app: &Arc<App>, agent: &Session, words: &str) -> Option<String> {
    let items: Vec<airband_core::commands::Command> = commands::of(app, &agent.agent.id).await.ok()?.items.into_iter().filter(|c| c.runs).collect();
    if items.is_empty() {
        return None;
    }
    let listed = items.iter().enumerate().map(|(i, c)| format!("{}. /{}: {}", i + 1, c.name, airband_core::prose::clip(&c.about, 120))).collect::<Vec<_>>().join("\n");
    let prompt = format!(
        "He said this aloud to a coding agent: \"{words}\"\n\nThe agent's tool can run these commands and skills, each listed under a number:\n{listed}\n\n\
         Does he ask the tool to run one of them by its name? He may say \"slash\" and the name, \"run the … command\", \
         or \"use the … skill\", and speech may write a name as words that sound like it. Work he asks for that one of \
         them happens to fit, or a remark about one, is not a request to run it. Reply with one line of JSON and nothing \
         else: {{\"command\": the number of the one he asks to run, or null, \"arguments\": what he gives it to work on, \
         in his words, without the words that name it, else \"\"}}"
    );
    let out = judge::ask_json::<Out>(&app.settings, prompt, Some(out_shape())).await.ok()?;
    let chosen = judge::listed(out.command, &items)?;
    log(format!("{}: his words read as one of its tool's commands", short_id(&agent.agent.id)));
    Some(format!("{}{}", airband_core::commands::typed(&chosen.name), out.arguments.trim()).trim_end().to_string())
}
