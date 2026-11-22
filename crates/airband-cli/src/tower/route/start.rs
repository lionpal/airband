//! Starting a new agent he asked for: where an idea goes when he named no place, and his answer to which.
use super::*;

/// A model's answer that picks one place from a numbered list, or none.
#[derive(Deserialize)]
pub(super) struct Picked {
    project: Option<usize>,
}

/// The shape of [`Picked`], as the model is held to it.
fn picked_shape() -> Value {
    json!({
        "type": "object",
        "properties": { "project": { "type": ["integer", "null"] } },
        "required": ["project"],
        "additionalProperties": false
    })
}

/// He was asked which place an idea belongs in, and has said something. The place his words name, or nothing
/// when they are not an answer to that. `Err` when no model could say: then nothing is known either way.
pub(super) async fn named_project(app: &Arc<App>, idea: &str, said: &str, projects: &[Place]) -> Result<Option<Place>, String> {
    let prompt = format!(
        "{name} wanted a new coding agent started on this: \"{idea}\"\nHe was asked which of his projects it is for:\n{list}\n\n\
         He now says: \"{said}\"\n\n\
         Does that answer the question, and if so with which project? An answer does little more than name one \
         (speech may write a name apart or misspell it). Anything that asks for other work or says something of \
         its own is a new message and not an answer, even when it mentions a project.\n\
         Reply with one line of JSON and nothing else: {{\"project\":<the number it is listed under, or null>}}\n\
         Null if he names none of them, rules one out without choosing another, or has gone on to \
         say something else.",
        name = app.cfg().name,
        list = projects.iter().enumerate().map(|(i, p)| format!("{}. {}", i + 1, p.name)).collect::<Vec<_>>().join("\n"),
    );
    let picked: Picked = judge::ask_json(&app.settings, "which project he named", prompt, Some(picked_shape())).await?;
    Ok(judge::listed(picked.project, projects).cloned())
}

/// Start the new agent he asked for. Where it goes was read with his words (`understand`): a place he named or
/// the model chose, or, with one place, that one. With none, he is asked which, and the idea is kept for his
/// answer.
pub(super) async fn start(app: &Arc<App>, radio: &str, text: &str, idea: Idea, projects: &[Place]) -> Reply {
    let say = |s: String, listen: bool| Reply { say: s, listen, ..Reply::default() };
    let place = match (&idea.project, projects) {
        (Some(place), _) => place.clone(),
        (None, _) if idea.task.is_empty() => return say("What should it do? Say new idea, then the idea.".into(), true),
        (None, []) => return say("There is no project to start it in yet. Run an agent in one first.".into(), false),
        (None, [only]) => only.clone(),
        (None, _) => {
            let question = config::random_hex(8);
            app.change().call(radio).unplaced_idea = Some((question.clone(), idea));
            log(format!("← radio: \"{text}\" → a new agent, project not clear"));
            return Reply { say: which_project(projects), listen: true, question: Some(question), ..Reply::default() };
        }
    };
    log(format!("← radio: \"{text}\" → launch in {}", place.name));
    launch(app, &place, &idea).await
}

/// The question when the place a new agent is for is not known.
fn which_project(projects: &[Place]) -> String {
    format!("Which project is that for? {}.", either(&projects.iter().map(|p| p.name.as_str()).collect::<Vec<_>>()))
}
