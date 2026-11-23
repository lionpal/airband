//! Starting a new agent he asked for: where an idea goes when he named no place, and his answer to which.
#![expect(clippy::disallowed_macros, reason = "the model's answer shape")]
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
    let say = |s: String, listen: bool| reply(s, listen);
    let place = match (&idea.project, projects) {
        (Some(place), _) => place.clone(),
        (None, _) if idea.task.is_empty() => return say(launch::what_to_do(None), true),
        (None, []) => return say("There is no project to start it in yet. Run an agent in one first.".into(), false),
        (None, [only]) => only.clone(),
        (None, _) => {
            let question = airband_core::ids::QuestionId::new(config::random_hex(8));
            app.change().call(radio).hold_unplaced(Unplaced { question: question.clone(), idea });
            log(format!("← radio: {} characters → a new agent, project not clear", text.chars().count()));
            return Reply { say: which_project(projects), listen: true, question: Some(question.clone()), ..Reply::default() };
        }
    };
    // several could start it and none is his usual: he is asked which, and the idea waits for his answer
    if let Err(launch::Unsettled::Which { what, among }) = launch::starter(app, &idea, None) {
        let question = airband_core::ids::QuestionId::new(config::random_hex(8));
        let say = launch::Unsettled::Which { what, among: among.clone() }.words();
        app.change().call(radio).hold_unstarted(Unstarted { question: question.clone(), asked: say.clone(), place, idea, among });
        log(format!("← radio: {} characters → a new agent, which launcher not clear", text.chars().count()));
        return Reply { say, listen: true, question: Some(question), ..Reply::default() };
    }
    log(format!("← radio: {} characters → launch in {}", text.chars().count(), place.name));
    launch(app, &place, &idea).await.unwrap_or_else(|refused| refused)
}

/// He was asked which of several ways of starting an agent should start one, and has said something. The one
/// his words name, or nothing when they are not an answer to that. `Err` when no model could say.
pub(super) async fn named_launcher(app: &Arc<App>, asked: &str, said: &str, among: &[(LauncherId, String)]) -> Result<Option<LauncherId>, String> {
    let prompt = format!(
        "{name} wanted a new coding agent started, and was asked: \"{asked}\" The choices:\n{list}\n\n\
         He now says: \"{said}\"\n\n\
         Does that answer the question, and if so with which choice? An answer does little more than name one \
         (speech may write a name apart or misspell it). Anything that asks for other work or says something of \
         its own is a new message and not an answer.\n\
         Reply with one line of JSON and nothing else: {{\"project\":<the number it is listed under, or null>}}\n\
         Null if he names none of them, rules one out without choosing another, or has gone on to say something else.",
        name = app.cfg().name,
        list = among.iter().enumerate().map(|(i, (_, n))| format!("{}. {n}", i + 1)).collect::<Vec<_>>().join("\n"),
    );
    let picked: Picked = judge::ask_json(&app.settings, "which launcher he named", prompt, Some(picked_shape())).await?;
    Ok(judge::listed(picked.project, among).map(|(id, _)| id.clone()))
}

/// The question when the place a new agent is for is not known.
/// A few are read out; more than that, and he is asked for its name.
fn which_project(projects: &[Place]) -> String {
    match projects {
        few @ [_, _] | few @ [_, _, _] => format!("Which project is that for? {}.", either(&few.iter().map(|p| p.name.as_str()).collect::<Vec<_>>())),
        _ => "Which project is that for? Say its name.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn several_launchers_and_none_usual_is_a_question_whose_idea_waits_for_his_answer() {
        let app = crate::tower::http::tests::app_with("which-launcher", serde_json::json!({ "launcher": "mine", "launchers": {
            "mine": { "harness": "tool-one", "argv": ["true"] },
            "c": { "harness": "tool-three", "argv": ["true"], "models": { "Nova": "nova-c" } },
            "d": { "harness": "tool-four", "argv": ["true"], "models": { "Nova": "nova-d" } },
        } }));
        let (c, d) = (LauncherId::new("c"), LauncherId::new("d"));
        let offer = |l: &LauncherId, id: &str| launch::Offered { launcher: l.clone(), choice: airband_core::tools::Choice { id: id.into(), name: "Nova".into() } };
        let place = Place { name: "shop".into(), folder: std::env::temp_dir().display().to_string() };
        let idea = Idea { project: Some(place.clone()), model: Some(Pick::Heard(launch::Heard { offered: vec![offer(&d, "nova-d"), offer(&c, "nova-c")] })), task: "tidy it".into(), ..Idea::default() };
        let reply = start(&app, "radio", "tidy it on nova", idea.clone(), std::slice::from_ref(&place)).await;
        assert_eq!(reply.say, "Which should start it on Nova: c or d?");
        assert!(reply.listen);
        let question = reply.question.expect("a question he can answer");
        let held = app.change().call("radio").unstarted_for(Some(question.as_str())).expect("the idea waits");
        assert_eq!((held.place, held.idea, held.among), (place, idea, vec![(c, "c".to_string()), (d, "d".to_string())]));
    }

    #[test]
    fn a_few_projects_are_read_out_and_more_are_asked_for_by_name() {
        let places = |names: &[&str]| names.iter().map(|n| Place { name: n.to_string(), folder: format!("/w/{n}") }).collect::<Vec<_>>();
        assert_eq!(which_project(&places(&["shop", "site"])), "Which project is that for? shop or site.");
        assert_eq!(which_project(&places(&["a", "b", "c", "d"])), "Which project is that for? Say its name.");
        assert_eq!(launch::what_to_do(Some("shop")), "What should it do? Say launch shop, then the task.");
    }
}
