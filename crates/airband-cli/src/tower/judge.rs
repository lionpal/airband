//! The first half of the dispatcher: how much of his attention a message deserves, and what to tell him of it.
//! Also the fuller version, for "say more". The model is the one chosen to read (`models`).
#![expect(clippy::disallowed_macros, reason = "model request shapes")]
use super::*;
use crate::providers::{By, Interface, Job, Offer};
use super::edges::models::Model;

mod brief;
mod chosen;
mod later;
mod models;
use brief::*;
pub(in crate::tower) use later::{JUDGE_NOW, judge_entry, judge_later};
pub(in crate::tower) use chosen::*;
pub(crate) use models::*;

/// A model that did not answer: why, and anything more for this computer's own log. The second part is never
/// sent to a phone.
pub(crate) struct Failed(pub Trouble, pub String);

impl From<Trouble> for Failed {
    fn from(trouble: Trouble) -> Failed {
        Failed(trouble, String::new())
    }
}

/// What a model wrote, read as JSON of one shape: the one place a model's answer is read. A service held to a
/// schema answers with the JSON alone. One asked in words may wrap it in a code fence or a sentence: the object
/// is taken from its first `{` to its last `}` and read whole. Nothing when there is no such object, or it is
/// not of this shape; the caller says so.
pub(super) fn json_of<T: serde::de::DeserializeOwned>(out: &str) -> Option<T> {
    let out = out.trim();
    let object = out.find('{').zip(out.rfind('}')).filter(|(a, b)| a < b).map(|(a, b)| &out[a..=b])?;
    serde_json::from_str(object).ok()
}

/// Ask the model and read its answer as JSON of one shape. `what` names the question for the trace: with
/// `AIRBAND_TRACE` set, what was asked and what came back are logged. Err says why there is no answer.
pub(super) async fn ask_json<T: serde::de::DeserializeOwned>(model: &dyn Model, what: &str, prompt: String, shape: Option<Value>) -> Result<T, String> {
    let traced = std::env::var_os("AIRBAND_TRACE").is_some().then(|| prompt.clone());
    let answer = model.ask(prompt, shape).await;
    if let Some(prompt) = traced {
        log(format!("{what} asked:\n{prompt}\nand was told: {answer:?}"));
    }
    json_of(&answer?).ok_or_else(|| "the answer was not JSON of the shape asked for".to_string())
}

/// What a list holds under the number a model answered with. The lists it is shown are numbered from one.
pub(super) fn listed<T>(n: Option<usize>, list: &[T]) -> Option<&T> {
    list.get(n?.checked_sub(1)?)
}

/// What the judge decided about one message.
pub(super) struct Judged {
    /// `say` is cut down from the message, so there is more to read: the model's verdict.
    pub(crate) cut: bool,
    pub(crate) kind: Kind,
    pub(crate) say: String,
    /// Answers he could give with one tap.
    pub(crate) options: Vec<String>,
    /// Which of them the agent itself recommended, if it said.
    pub(crate) recommended: Option<u32>,
    /// What the message holds that he should be able to read whole, in two or three words, if anything.
    pub(crate) read: Option<String>,
    /// It stopped only to wait for work of its own that is still running: it is still at work.
    pub(crate) background: bool,
    /// The agent says it will take this answer only from him in its own session, and not from a message that
    /// reaches it another way: an answer sent from a radio to an agent the tower does not run would not count.
    pub(crate) own_session: bool,
}

impl Judged {
    /// The message as its agent wrote it, of this kind, with nothing read into it.
    fn as_written(item: &Item, kind: Kind) -> Judged {
        Judged { kind, say: spoken(&item.full), cut: false, options: Vec::new(), recommended: None, read: None, background: false, own_session: false }
    }
}

/// The tower's main job: decide how much of his attention a message deserves, what to tell him of it, and, when
/// it is a question he can answer by picking, what the picks are.
pub(super) async fn prepare(settings: &Settings, item: &Item) -> Judged {
    if item.notice {
        return Judged::as_written(item, Kind::NeedsYou);
    }
    let parsed = ask_json::<Answer>(settings, "reading a message", reading(&settings.current(), item), Some(answer_shape())).await;
    match parsed {
        // a kind the model made up is no verdict: he gets the message as written, below
        Ok(o) if !o.say.trim().is_empty() && o.kind != Kind::Unknown => judged(item, o),
        other => {
            // No model answered. Nothing is guessed from the words: he gets the message as the agent wrote it,
            // marked as read by no model unless its harness said what it is.
            if let Err(e) = other {
                log(format!("no summary, giving him the message as written: {e}"));
            }
            Judged::as_written(item, item.kind.unwrap_or(Kind::Unjudged))
        }
    }
}

/// What the model reading a message for him answers.
#[derive(Deserialize)]
struct Answer {
    kind: Kind,
    say: String,
    #[serde(default)]
    options: Vec<String>,
    #[serde(default)]
    recommended: Option<u32>,
    #[serde(default)]
    read: Option<String>,
    #[serde(default)]
    background: bool,
    #[serde(default)]
    own_session: bool,
    #[serde(default)]
    as_written: bool,
}

/// The question put to the model that reads what an agent wrote when it stopped: how much it deserves his
/// attention, what to tell him, the choices he could answer with, and what to offer him to read whole.
fn reading(cfg: &Config, item: &Item) -> String {
    format!(
        "{name} runs several coding agents at once and is away from his desk. You are his dispatcher: you decide \
         what reaches him and in how few words. Below is what one agent wrote when it stopped.\n\n\
         First, how much it deserves his attention:\n\
         - \"needs-you\": the agent is stopped and will not go on by itself: it is waiting for his answer or for \
         something only he can do, or it was halted (a usage limit, an error, a missing login). Also when \
         something broke, was lost or went wrong that he has to know now. It is a question to him or a stop, \
         never a report that happens to mention something he will have to do somewhere else in his own time (pay a \
         bill, create an account, try the new build): the agent that has answered him and gone on, or finished, \
         is not waiting, and that is a \"report\" however much is left for him to do.\n\
         - \"report\": it finished a piece of work, or found or decided something he would want to know: a result, \
         a choice he might disagree with, something it left undone.\n\
         - \"stalled\": it has stopped part way. It says what it will do next (\"Next I will…\", \"I'll now…\"), \
         or that it is starting or carrying on, but this is where it stopped: the work is not finished and it \
         asks him nothing. Remember that it wrote this as it stopped, so nothing it says it is about to do is \
         being done. It needs a nudge to go on, not an answer. The exception is an agent waiting for something \
         that will bring it back by itself (a build, a test run, a background task): that is \"fyi\".\n\
         - \"fyi\": he would not miss it. A message whose whole content is that there is nothing to tell \
         (\"nothing new\", \"no change since last check\", \"still waiting\") is always this, never \"stalled\" \
         or \"report\": an agent that checks on a timer and found nothing has not stopped short of anything. \
         Also: progress chatter, an acknowledgement (\"OK, starting\"), a plan it is \
         about to carry out anyway, a restatement of what he just told it, an interim note while work goes on.\n\
         When unsure between two, pick the lower one, unless the agent is waiting on him or has stopped with \
         work it said it would still do: a message that ends by saying what comes next (\"next up is the \
         importer\", \"then I will wire the tests\") from an agent that has stopped is \"stalled\", however \
         much it reports before that.\n\n\
         Then what to tell him, in plain spoken English. He will not read much: he glances. One short sentence, \
         14 words at most. If the agent is asking him something, the sentence is the question itself, as \
         briefly as it can be put (\"Two questions are too easy. Rewrite or leave?\"); how it got there is \
         one tap away and does not belong here.\n\
         The one exception is findings he asked for, so check for it before writing anything: look at the last \
         thing he told this agent, given below. If that was to investigate, research, review, compare, audit or \
         explain something, and this message is the answer, a headline would throw the answer away. Then \"say\" \
         is an executive summary instead: three to five short sentences, between 40 and 90 words, the conclusion \
         first, then the findings that matter with their numbers as digits, then what he has to decide if \
         anything. It is read aloud to him, so it is sentences, never a list.\n\
         The same longer form is for a message that leaves him several things to decide or do: he cannot act on \
         a count. Never say that there are some number of things (\"four decisions needed\", \"three issues found\") \
         without saying what each one is, in a few words each: \"Before going public: delete the old logs, pick a \
         commit email, decide on the account code.\" If naming them takes more than one sentence, take the room.\n\
         - Keep only: the result; anything left undone or gone wrong; the one thing he must decide.\n\
         - Always keep anything the work is waiting on that only he can do (a request he has to file, an approval, \
         a payment, a sign-in, a setting only his account can change, a choice he has not made), whether or not \
         the agent has stopped for it: an agent that went on with other work has still left that part blocked on \
         him. Say it in its own sentence, beginning \"Waiting on you:\", with what he has to do and what it \
         unblocks (\"Waiting on you: approve the payment provider's form, before refunds can go out.\"). \
         If there are several, name each. This comes before detail about what was done.\n\
         - Always keep what the work left outside the conversation, each with the number or name that finds it: \
         a push or commit (its short id), a pull request or ticket (its number), a build or release (its number or \
         version, and where it went), a deploy (to what), a published page or file, a message sent to someone. \
         These are what he will go and look for, so they outrank detail about how the work was done. Say plainly \
         when one of them was made but not finished (built but not uploaded, committed but not pushed).\n\
         - If the agent needs an answer from him, end with that question, with the options if it gave any. \
         The question does not stand in for the rest: a message that reports finished work and then asks \
         something is told as the news first (what is now done, live, moved, spent or broken) and the question \
         last. He must not lose the news because a question came after it. Each finished thing is news of its own: \
         \"the bug is fixed, and the refunds are ready to send\" keeps both, and does not shrink to the one the \
         question is about. Take the longer form for it.\n\
         - No file paths, code, lists or markdown, and no identifiers other than those just named. Say \"the checker script\", not its filename.\n\
         - Speak as the agent, in the first person. No greeting, no padding (\"nothing needed from you\", \"let me know\").\n\
         - Say only what the message says. Add no fact, date or reassurance that is not in it.\n\n\
         Last, only when it is \"needs-you\" and he could answer by choosing: the choices, as button labels. At \
         most three, one to three words each, in the order the agent gave them. \"recommended\" is the position \
         (0 for the first) of the choice the AGENT ITSELF said it recommends or would do; null if it expressed \
         no preference. Never pick one yourself: he must be able to trust that a marked choice is the agent\'s \
         advice. A yes-or-no question gets [\"Yes\",\"No\"]. \
         \"Rewrite them or leave them?\" gets [\"Rewrite\",\"Leave\"]. If the answer needs his own words, or he \
         has to go and do something, give none.\n\n\
         And \"read\": when the message itself contains something he asked to be shown, or would have to read \
         whole before he could decide (the draft of an email, a plan, a list of what will change, the text of \
         a page), name that thing in two or three words (\"the refund email\", \"the migration plan\"). He is \
         then offered the whole message to read under that name. A line or two that says everything needs \
         nothing to read: null.\n\n\
         And \"own_session\": true only when the agent says it will not take the answer as a message passed on \
         to it (relayed by another session, or sent through Airband) and that he must give it himself in the \
         agent\'s own session or terminal. Then give no choices, and say in \"say\" what it is waiting to be \
         told, without telling him to reply \"here\". Otherwise false.\n\n\
         And \"as_written\": true only when the whole message is already one or two short plain sentences that \
         can be shown to him exactly as the agent wrote them, with nothing lost and nothing to cut: no list, no \
         code, no file path, no table. Then \"say\" is those words. Otherwise false.\n\n\
         Reply with one line of JSON and nothing else: \
         {{\"kind\":\"needs-you\" or \"report\" or \"stalled\" or \"fyi\",\"say\":\"...\",\"options\":[],\"recommended\":null,\"read\":null,\"background\":false,\"own_session\":false,\"as_written\":false}}\n\
         \"background\": true when it has stopped only to wait for work of its own that is still running and will \
         bring it back by itself (a build, a test run, a background agent or task it started): it is still at work.\n\n\
         {asked}<message>\n{msg}\n</message>",
        asked = item.asked.as_deref().map(|a| format!("The last thing he told this agent was:\n<asked>\n{}\n</asked>\nIf the message below says that is done, or cannot be done, that is at least a report.\n\n", airband_core::prose::tail(a, 2000))).unwrap_or_default(),
        name = cfg.name,
        msg = airband_core::prose::tail(&item.full, 12000)
    )
}

/// The shape of the answer, for a service that can be held to one. The words come before the verdict: a
/// small model that has just written what the message says judges it better than one that judges first.
fn answer_shape() -> Value {
    json!({
        "type": "object",
        "properties": {
            "say": { "type": "string" },
            "kind": { "type": "string", "enum": ["needs-you", "report", "stalled", "fyi"] },
            "options": { "type": "array", "items": { "type": "string" }, "maxItems": 3 },
            "recommended": { "type": ["integer", "null"] },
            "read": { "type": ["string", "null"] },
            "background": { "type": "boolean" },
            "own_session": { "type": "boolean" },
            "as_written": { "type": "boolean" }
        },
        "required": ["say", "kind", "options", "recommended", "read", "background", "own_session", "as_written"],
        "additionalProperties": false
    })
}

/// The model's answer, as what he is shown.
fn judged(item: &Item, o: Answer) -> Judged {
    let judged = o.kind;
    // His agent's own words beat a rewording of them, when the model that read the message says they
    // are already short and plain enough to show as they are. That is its verdict, never a guess made
    // here from the look of the text. Anything longer is cut down, because a card he has to read
    // through is the thing this exists to spare him.
    let say = if o.as_written { spoken(&item.full) } else { spoken(&o.say) };
    // a harness that knows its agent is blocked outranks the judge
    let kind = item.kind.unwrap_or(judged);
    let own_session = o.own_session && kind == Kind::NeedsYou;
    let options = if kind == Kind::NeedsYou && !own_session { o.options.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty() && c.chars().count() <= 24).take(3).collect() } else { Vec::new() };
    let recommended = o.recommended.filter(|i| (*i as usize) < options.len());
    // only for a message with more in it than its line: a name of a few words, and nothing for chatter
    // cut down by the model: its own verdict, never read from the words
    let cut = !o.as_written;
    let read = o.read.map(|r| r.trim().to_string()).filter(|r| !r.is_empty() && r.chars().count() <= 32 && kind != Kind::Fyi && cut);
    Judged { kind, say, cut, options, recommended, read, background: o.background && kind != Kind::NeedsYou, own_session }
}

/// An agent has been at work for a while without a word. From the labels of its steps: is there something he
/// would want to know now, before it reports? One sentence if so. The labels say what it set out to do, not
/// how it went, and the sentence must not claim more.
pub(super) async fn progress(model: &dyn Model, name: &str, steps: &[String]) -> Option<String> {
    let prompt = format!(
        "Below are the steps a coding agent has taken, in order, since it last said anything to {name}, who is away from its screen. \
         Each line is the agent's own label for a step: what it set out to do, not whether it worked.\n\n\
         Decide whether anything in them is something he would go and look for, or would want to know before the agent reports: \
         something sent outside the conversation or made to last (a push or commit, a build or release uploaded, a deploy, \
         a pull request or ticket, a message sent to someone, something installed or restarted on a machine, something deleted \
         or moved for good), or a sign the work has hit trouble (the same thing tried again and again, something undone, \
         a hunt for why something failed). Reading, searching, editing and running tests along the way are not.\n\n\
         Reply with one line of JSON and nothing else: {{\"tell\":true or false,\"say\":\"…\"}}\n\
         \"tell\" is false when nothing in them is, and \"say\" is then empty. Otherwise \"say\" is one sentence of at most 24 words, \
         in the first person as the agent, saying what it is doing or \
         has set going, with any number or name from the labels that finds it (a build number, a commit id, where a deploy goes). \
         Do not say that anything succeeded, finished or failed: the labels do not tell. No file paths, no markdown.\n\n<steps>\n{steps}\n</steps>",
        steps = steps.join("\n")
    );
    #[derive(Deserialize)]
    struct Out {
        tell: bool,
        #[serde(default)]
        say: String,
    }
    let shape = json!({
        "type": "object",
        "properties": { "tell": { "type": "boolean" }, "say": { "type": "string" } },
        "required": ["tell", "say"],
        "additionalProperties": false
    });
    let out: Out = ask_json(model, "what to tell him of its progress", prompt, Some(shape)).await.ok()?;
    let said = out.say.trim();
    (out.tell && !said.is_empty() && said.chars().count() < 400).then(|| said.to_string())
}

/// He answered a request for permission in his own words. Did he say yes? `Some(true)` only for a plain yes;
/// `Some(false)` for anything that is not one (a no, a condition, another instruction); `None` when his words
/// do not settle it, or no model answered: then nothing is decided and he is asked for a yes or a no.
///
/// The model sees his words only. The request is text the asking agent wrote, and nothing an agent writes is
/// put beside the one question whose answer lets a command run.
pub(super) async fn consent(model: &dyn Model, name: &str, said: &str) -> Option<bool> {
    let prompt = format!(
        "A coding agent asked {name} for permission to do one thing on his computer. He was shown exactly what, \
         and answered in his own words.\n\nHis answer:\n<answer>\n{said}\n</answer>\n\n\
         Reply with one line of JSON and nothing else, {{\"verdict\": one of \"allow\", \"refuse\", \"unclear\"}}:\n\
         - allow: he plainly says yes to exactly what was asked, with no condition and no change.\n\
         - refuse: he says no, or tells it what to do instead or first, or attaches a condition or a change \
         (\"run it on staging first\", \"not that folder\", \"only the tests\", \"yes but skip the migration\"). An \
         instruction of his own is a refusal of what was asked: it goes back to the agent as the reason.\n\
         - unclear: only when his words give the agent nothing to go on: he hands the choice back (\"whatever you \
         think\"), asks a question, or is talking about something else.\n\
         When in doubt between allow and anything else, it is not allow.",
        said = airband_core::prose::tail(said, 1000)
    );
    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum Verdict {
        Allow,
        Refuse,
        Unclear,
    }
    #[derive(Deserialize)]
    struct Out {
        verdict: Verdict,
    }
    let shape = json!({
        "type": "object",
        "properties": { "verdict": { "type": "string", "enum": ["allow", "refuse", "unclear"] } },
        "required": ["verdict"],
        "additionalProperties": false
    });
    // anything that is not exactly this shape with one of these three values settles nothing
    match ask_json::<Out>(model, "whether his words allow it", prompt, Some(shape)).await.ok()?.verdict {
        Verdict::Allow => Some(true),
        Verdict::Refuse => Some(false),
        Verdict::Unclear => None,
    }
}

/// The fuller spoken version of one message, from `model`. Err says why there is none.
pub(super) async fn detail(model: &dyn Model, name: &str, item: &Item) -> Result<String, String> {
    let prompt = format!(
        "Below is a message a coding agent wrote to {name}, who is driving. He heard this short version: \"{say}\" and asked for more.\n\
         Give the fuller spoken version: four to six sentences of plain spoken English covering what was done, what was found, and any open decision. \
         No file paths, code, lists or markdown. Do not repeat the short version word for word. Output only the words to be spoken.\n\n<message>\n{msg}\n</message>",
        say = item.say.as_deref().unwrap_or(""),
        msg = airband_core::prose::tail(&item.full, 12000)
    );
    model.ask(prompt, None).await.map(|t| spoken(&t))
}

/// The fuller spoken version of a logged message, made once and kept. When the model has none to give, he
/// gets the message itself this time, and nothing is kept, so the next time he asks it is made again. Why
/// the model gave none he is told where he is told of the model that reads (`Settings::asked`).
pub(super) async fn detail_of(app: &Arc<App>, id: &str) -> Option<String> {
    detail_by(app, &app.settings, id).await
}

pub(super) async fn detail_by(app: &Arc<App>, model: &dyn Model, id: &str) -> Option<String> {
    let entry = app.read().log.iter().find(|e| e.id == id).cloned()?;
    if let Some(d) = entry.detail {
        return Some(d);
    }
    let item = Item { id: entry.id.clone(), notice: false, full: entry.full.clone(), kind: entry.kind, say: entry.short.clone(), asked: None };
    let name = app.cfg().name.clone();
    match detail(model, &name, &item).await {
        Ok(d) => {
            let mut inner = app.change();
            if let Some(e) = inner.log.iter_mut().find(|e| e.id == id) {
                e.detail = Some(d.clone());
                inner.unsaved.borrow_mut().insert(id.to_string());
            }
            Some(d)
        }
        Err(e) => {
            log(format!("no detail, reading the message instead: {e}"));
            Some(airband_core::prose::clip(&spoken(&item.full), 900))
        }
    }
}

#[cfg(test)]
mod detail_tests {
    use super::*;
    use crate::tower::edges::models::FakeModel;

    #[tokio::test]
    async fn a_detail_the_model_could_not_make_is_not_kept_and_is_asked_for_again() {
        let app = crate::tower::http::tests::app("detail");
        app.change().log.push(Entry { id: "m1".into(), full: "The build passed after the fix.".into(), ..Entry::default() });
        let failing = FakeModel::answers(Err("no answer"));
        assert_eq!(detail_by(&app, &failing, "m1").await.as_deref(), Some("The build passed after the fix."));
        assert!(app.read().log[0].detail.is_none(), "nothing made is kept");
        let working = FakeModel::answers(Ok("It passed, after one fix."));
        assert_eq!(detail_by(&app, &working, "m1").await.as_deref(), Some("It passed, after one fix."));
        assert_eq!(working.asked.load(std::sync::atomic::Ordering::SeqCst), 1, "asked again once the first gave none");
        // kept now: never asked again
        assert_eq!(detail_by(&app, &failing, "m1").await.as_deref(), Some("It passed, after one fix."));
        assert_eq!(failing.asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn his_words_allow_a_request_only_on_a_plain_yes_from_the_model_and_nothing_else_settles_it() {
        for (answer, settles) in [(r#"{"verdict":"allow"}"#, Some(true)), (r#"{"verdict":"refuse"}"#, Some(false)), (r#"{"verdict":"unclear"}"#, None), (r#"{"verdict":"maybe"}"#, None), ("yes", None)] {
            let model = FakeModel::answers(Ok(answer));
            assert_eq!(consent(&model, "Charles", "go ahead").await, settles, "{answer}");
        }
        assert_eq!(consent(&FakeModel::answers(Err("no answer")), "Charles", "go ahead").await, None);
    }

    #[tokio::test]
    async fn progress_is_told_only_when_the_model_says_to_and_gives_a_sentence() {
        let told = FakeModel::answers(Ok(r#"{"tell":true,"say":"I pushed build 41."}"#));
        assert_eq!(progress(&told, "Charles", &["push".into()]).await.as_deref(), Some("I pushed build 41."));
        for quiet in [r#"{"tell":false,"say":""}"#, r#"{"tell":true,"say":"  "}"#, "not json"] {
            assert_eq!(progress(&FakeModel::answers(Ok(quiet)), "Charles", &["read".into()]).await, None, "{quiet}");
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn why_a_model_did_not_answer_is_told_from_the_status_and_never_in_the_services_words() {
        assert_eq!((Trouble::of_status(401, None), Trouble::of_status(403, None)), (Trouble::WrongKey, Trouble::WrongKey));
        assert_eq!((Trouble::of_status(402, None), Trouble::of_status(429, None)), (Trouble::Refused, Trouble::Refused));
        assert_eq!((Trouble::of_status(404, None), Trouble::of_status(503, None)), (Trouble::NotFound, Trouble::Unexpected { status: 503 }));
        // one sentence each, all different, each whole, and the same ones a key that failed its test is told in
        let all = [Trouble::Unreachable, Trouble::WrongKey, Trouble::Refused, Trouble::NotFound, Trouble::Unexpected { status: 500 }, Trouble::NoAnswer, Trouble::Program, Trouble::TooSlow, Trouble::NotSet];
        let said: std::collections::HashSet<String> = all.iter().map(|t| t.said("Maker")).collect();
        assert!(said.len() == all.len() && said.iter().all(|s| s.ends_with('.')));
        assert!(all.iter().all(|t| t.said_of_key("Maker").starts_with(&t.said("Maker"))));
        // what goes to a phone is the kind of trouble alone; what is for this computer's log stays here
        let failed = Failed(Trouble::Program, "the program printed sk-not-for-a-phone".into());
        assert!(!serde_json::to_string(&failed.0).unwrap().contains("sk-"));
    }

    use super::*;

    #[test]
    fn every_reader_in_the_catalogue_is_one_this_tower_has() {
        let mut named: Vec<&str> = READERS.iter().map(|r| r.is.name).collect();
        named.sort();
        named.dedup();
        assert_eq!(named.len(), READERS.len(), "an interface is in the table twice");
        for p in &airband_core::catalog::built().providers {
            for o in p.offers.iter().filter(|o| o.job == Job::Summary) {
                assert!(reads(o), "{}: nothing here reads by \"{}\", or its row does not tell it what it needs or asks more than it can do", p.id, o.by.interface);
                // one that takes a key says how the key is presented
                assert!(!p.key || o.by.auth().is_some(), "{}: its row does not say how its key is presented", p.id);
            }
        }
        // what starts a program here is exactly what a later catalogue may not bring or change
        let mut runs: Vec<&str> = READERS.iter().filter(|r| r.runs).map(|r| r.is.name).collect();
        let mut held: Vec<&str> = airband_core::catalog::BUILT_IN_ONLY.to_vec();
        runs.sort();
        held.sort();
        assert_eq!(runs, held);
        // an interface this tower has, asked for something it cannot do, and one it does not have: not offered
        let odd: Offer = serde_json::from_value(json!({ "id": "new", "job": "summary", "by": { "interface": "chat" }, "label": "New", "needs": ["auth-header"] })).unwrap();
        let new: Offer = serde_json::from_value(json!({ "id": "new", "job": "summary", "by": { "interface": "something-new" }, "label": "New" })).unwrap();
        assert!(!reads(&odd) && !reads(&new));
        // every offer for reading, as this program lists them, is one it can do; listening and speaking are
        // the phone's, and nothing is offered for them here
        let job = Job::Summary;
        assert!(crate::providers::in_order(job).len() >= 3 && crate::providers::in_order(job).iter().all(|(_, o)| crate::providers::speaks(o)));
        assert_eq!(crate::providers::in_order(job).len(), airband_core::providers::in_order(job).len(), "an offer compiled in is one this build cannot do");
        assert!(crate::providers::in_order(Job::Voice).is_empty() && crate::providers::in_order(Job::Ears).is_empty());
    }
}
