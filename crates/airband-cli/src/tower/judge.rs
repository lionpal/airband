//! The first half of the dispatcher: how much of his attention a message deserves, and what to tell him of it.
//! Also the fuller version, for "say more". The model is whatever command the configuration names.
use super::*;

/// One question to a model behind an API key, off the async threads. `read` finds the answer in what came back.
async fn ask_api(url: String, header: (&'static str, String), body: Value, read: fn(&Value) -> Option<String>) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let mut res = airband_radio::net::agent(Duration::from_secs(60)).post(&url).header(header.0, &header.1).send_json(&body).map_err(|e| format!("the model could not be reached: {e}"))?;
        let status = res.status().as_u16();
        let v: Value = res.body_mut().read_json().map_err(|e| e.to_string())?;
        if status != 200 {
            return Err(format!("the model refused ({status}): {}", v["error"]["message"].as_str().unwrap_or("no reason given").chars().take(160).collect::<String>()));
        }
        read(&v).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).ok_or_else(|| "the model answered with nothing".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The JSON object in what a model wrote, which may have words or a code fence around it. None when there is
/// no object there (a closing brace before any opening one is not one).
pub(super) fn json_in(out: &str) -> Option<&str> {
    let (a, b) = (out.find('{')?, out.rfind('}')?);
    out.get(a..=b)
}

/// Ask the model that reads for the tower. If it does not answer, the second one is asked, when there is one.
pub(super) async fn ask_model(cfg: &Config, prompt: String) -> Result<String, String> {
    match ask(&cfg.summarizer, prompt.clone()).await {
        Ok(text) => Ok(text),
        Err(first) => match &cfg.summarizer_fallback {
            Some(second) => {
                log(format!("the model did not answer ({first}); asking the second"));
                ask(second, prompt).await.map_err(|e| format!("{first}; and the second: {e}"))
            }
            None => Err(first),
        },
    }
}

async fn ask(model: &Summarizer, prompt: String) -> Result<String, String> {
    let argv = match model {
        Summarizer::Command { argv } => argv,
        Summarizer::Gemini { key, model } => {
            let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent");
            let body = json!({ "contents": [{ "parts": [{ "text": prompt }] }], "generationConfig": { "temperature": 0 } });
            return ask_api(url, ("x-goog-api-key", key.clone()), body, |v| Some(v["candidates"][0]["content"]["parts"].as_array()?.iter().filter_map(|p| p["text"].as_str()).collect::<String>())).await;
        }
        Summarizer::Openai { url, key, model } => {
            let body = json!({ "model": model, "messages": [{ "role": "user", "content": prompt }], "temperature": 0 });
            return ask_api(format!("{}/chat/completions", url.trim_end_matches('/')), ("authorization", format!("Bearer {key}")), body, |v| v["choices"][0]["message"]["content"].as_str().map(String::from)).await;
        }
        Summarizer::None => return Err("no summarizer is configured".into()),
    };
    let (program, args) = argv.split_first().ok_or("the summarizer command is empty")?;
    // on Windows a command like `claude` is a .cmd shim, which only the shell can start
    let mut cmd = if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(program);
        c
    } else {
        tokio::process::Command::new(program)
    };
    let mut child = cmd
        .args(args)
        .current_dir(config::dir())
        .env("AIRBAND_INTERNAL", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("cannot start {program}: {e}"))?;
    let mut stdin = child.stdin.take().unwrap();
    tokio::spawn(async move {
        let _ = stdin.write_all(prompt.as_bytes()).await;
    });
    let out = tokio::time::timeout(Duration::from_secs(90), child.wait_with_output()).await.map_err(|_| format!("{program} took over 90 seconds"))?.map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || text.is_empty() {
        return Err(format!("{program} failed: {}", String::from_utf8_lossy(&out.stderr).chars().take(200).collect::<String>()));
    }
    Ok(text)
}

pub(super) fn tail(text: &str, max_chars: usize) -> String {
    let n = text.chars().count();
    text.chars().skip(n.saturating_sub(max_chars)).collect()
}

/// What the judge decided about one message.
pub(super) struct Judged {
    pub kind: Kind,
    pub say: String,
    /// Answers he could give with one tap.
    pub options: Vec<String>,
    /// Which of them the agent itself recommended, if it said.
    pub recommended: Option<u32>,
    /// What the message holds that he should be able to read whole, in two or three words, if anything.
    pub read: Option<String>,
    /// It stopped only to wait for work of its own that is still running: it is still at work.
    pub background: bool,
}

/// The tower's main job: decide how much of his attention a message deserves, what to tell him of it, and, when
/// it is a question he can answer by picking, what the picks are.
pub(super) async fn prepare(cfg: &Config, item: &Item) -> Judged {
    if item.notice {
        return Judged { kind: Kind::NeedsYou, say: spoken(&item.full), options: Vec::new(), recommended: None, read: None, background: false };
    }
    let prompt = format!(
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
         - Always keep what the work left outside the conversation, each with the number or name that finds it: \
         a push or commit (its short id), a pull request or ticket (its number), a build or release (its number or \
         version, and where it went), a deploy (to what), a published page or file, a message sent to someone. \
         These are what he will go and look for, so they outrank detail about how the work was done. Say plainly \
         when one of them was made but not finished (built but not uploaded, committed but not pushed).\n\
         - If the agent needs an answer from him, end with that question, with the options if it gave any. \
         The question does not stand in for the rest: a message that reports finished work and then asks \
         something is told as the news first (what is now done, live, moved, spent or broken) and the question \
         last. He must not lose the news because a question came after it. Take the longer form for it.\n\
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
         Reply with one line of JSON and nothing else: \
         {{\"kind\":\"needs-you\" or \"report\" or \"stalled\" or \"fyi\",\"say\":\"...\",\"options\":[],\"recommended\":null,\"read\":null,\"background\":false}}\n\
         \"background\": true when it has stopped only to wait for work of its own that is still running and will \
         bring it back by itself (a build, a test run, a background agent or task it started): it is still at work.\n\n\
         {asked}<message>\n{msg}\n</message>",
        asked = item.asked.as_deref().map(|a| format!("The last thing he told this agent was:\n<asked>\n{}\n</asked>\nIf the message below says that is done, or cannot be done, that is at least a report.\n\n", tail(a, 2000))).unwrap_or_default(),
        name = cfg.name,
        msg = tail(&item.full, 12000)
    );
    #[derive(Deserialize)]
    struct Out {
        kind: String,
        say: String,
        #[serde(default)]
        options: Vec<String>,
        #[serde(default)]
        recommended: Option<u32>,
        #[serde(default)]
        read: Option<String>,
        #[serde(default)]
        background: bool,
    }
    let parsed = ask_model(cfg, prompt).await.and_then(|out| {
        serde_json::from_str::<Out>(json_in(&out).ok_or("no JSON in the answer")?).map_err(|e| e.to_string())
    });
    match parsed {
        Ok(o) if !o.say.trim().is_empty() => {
            let judged = match o.kind.as_str() {
                "needs-you" | "question" => Kind::NeedsYou,
                "fyi" => Kind::Fyi,
                "stalled" => Kind::Stalled,
                _ => Kind::Report,
            };
            // already one short plain line: his agent's own words beat a rewording of them. Anything longer is
            // cut down, because a card he has to read through is the thing this exists to spare him
            let say = if reads_as_written(&item.full, 110) { spoken(&item.full) } else { spoken(&o.say) };
            // a harness that knows its agent is blocked outranks the judge
            let kind = item.kind.unwrap_or(judged);
            let options = if kind == Kind::NeedsYou { o.options.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty() && c.chars().count() <= 24).take(3).collect() } else { Vec::new() };
            let recommended = o.recommended.filter(|i| (*i as usize) < options.len());
            // only for a message with more in it than its line: a name of a few words, and nothing for chatter
            let read = o.read.map(|r| r.trim().to_string()).filter(|r| !r.is_empty() && r.chars().count() <= 32 && kind != Kind::Fyi && spoken(&item.full) != say);
            Judged { kind, say, options, recommended, read, background: o.background && kind != Kind::NeedsYou }
        }
        other => {
            // No model answered. Nothing is guessed from the words: he gets the message as the agent wrote it,
            // marked as a report unless its harness said what it is.
            if let Err(e) = other {
                log(format!("no summary, giving him the message as written: {e}"));
            }
            Judged { kind: item.kind.unwrap_or(Kind::Report), say: spoken(&item.full), options: Vec::new(), recommended: None, read: None, background: false }
        }
    }
}

/// An agent has been at work for a while without a word. From the labels of its steps: is there something he
/// would want to know now, before it reports? One sentence if so. The labels say what it set out to do, not
/// how it went, and the sentence must not claim more.
pub(super) async fn progress(cfg: &Config, steps: &[String]) -> Option<String> {
    let prompt = format!(
        "Below are the steps a coding agent has taken, in order, since it last said anything to {name}, who is away from its screen. \
         Each line is the agent's own label for a step: what it set out to do, not whether it worked.\n\n\
         Decide whether anything in them is something he would go and look for, or would want to know before the agent reports: \
         something sent outside the conversation or made to last (a push or commit, a build or release uploaded, a deploy, \
         a pull request or ticket, a message sent to someone, something installed or restarted on a machine, something deleted \
         or moved for good), or a sign the work has hit trouble (the same thing tried again and again, something undone, \
         a hunt for why something failed). Reading, searching, editing and running tests along the way are not.\n\n\
         If nothing is, answer with the single word: none\n\
         Otherwise answer with one sentence of at most 24 words, in the first person as the agent, saying what it is doing or \
         has set going, with any number or name from the labels that finds it (a build number, a commit id, where a deploy goes). \
         Do not say that anything succeeded, finished or failed: the labels do not tell. No file paths, no markdown.\n\n<steps>\n{steps}\n</steps>",
        name = cfg.name,
        steps = steps.join("\n")
    );
    let said = ask_model(cfg, prompt).await.ok()?;
    let said = said.trim().trim_matches('"').trim();
    (!said.is_empty() && !said.trim_end_matches('.').eq_ignore_ascii_case("none") && said.chars().count() < 400).then(|| said.to_string())
}

/// He answered a request for permission in his own words. Did he say yes? `Some(true)` only for a plain yes;
/// `Some(false)` for anything that is not one (a no, a condition, another instruction); `None` when his words
/// do not settle it, or no model answered: then nothing is decided and he is asked for a yes or a no.
pub(super) async fn consent(cfg: &Config, request: &str, said: &str) -> Option<bool> {
    let prompt = format!(
        "A coding agent asked {name} for permission to do something, and he answered in his own words.\n\n\
         The request, as he was shown it:\n<request>\n{request}\n</request>\n\nHis answer:\n<answer>\n{said}\n</answer>\n\n\
         Reply with one word and nothing else:\n\
         - allow: he plainly says yes to exactly what was asked, with no condition and no change.\n\
         - refuse: he says no, or tells it what to do instead or first, or attaches a condition or a change \
         (\"run it on staging first\", \"not that folder\", \"only the tests\", \"yes but skip the migration\"). An \
         instruction of his own is a refusal of what was asked: it goes back to the agent as the reason.\n\
         - unclear: only when his words give the agent nothing to go on: he hands the choice back (\"whatever you \
         think\"), asks a question, or is talking about something else.\n\
         When in doubt between allow and anything else, it is not allow.",
        name = cfg.name,
        request = tail(request, 2000),
        said = tail(said, 1000)
    );
    let out = ask_model(cfg, prompt).await.ok()?;
    if std::env::var_os("AIRBAND_TRACE").is_some() {
        log(format!("asked whether \"{said}\" allows it, and was told: {out}"));
    }
    match out.trim().trim_matches(|c: char| !c.is_ascii_alphabetic()).to_ascii_lowercase().as_str() {
        "allow" => Some(true),
        "refuse" => Some(false),
        _ => None,
    }
}

pub(super) async fn detail(cfg: &Config, item: &Item) -> String {
    if reads_as_written(&item.full, 500) {
        return spoken(&item.full);
    }
    let prompt = format!(
        "Below is a message a coding agent wrote to {name}, who is driving. He heard this short version: \"{say}\" and asked for more.\n\
         Give the fuller spoken version: four to six sentences of plain spoken English covering what was done, what was found, and any open decision. \
         No file paths, code, lists or markdown. Do not repeat the short version word for word. Output only the words to be spoken.\n\n<message>\n{msg}\n</message>",
        name = cfg.name,
        say = item.say.as_deref().unwrap_or(""),
        msg = tail(&item.full, 12000)
    );
    match ask_model(cfg, prompt).await {
        Ok(t) => spoken(&t),
        Err(e) => {
            log(format!("no detail, reading the message instead: {e}"));
            spoken(&item.full).chars().take(900).collect()
        }
    }
}


/// The fuller spoken version of a logged message, made once and kept.
pub(super) async fn detail_of(app: &Arc<App>, id: &str) -> Option<String> {
    let entry = app.inner.lock().unwrap().log.iter().find(|e| e.id == id).cloned()?;
    if let Some(d) = entry.detail {
        return Some(d);
    }
    let item = Item { id: entry.id.clone(), agent_id: String::new(), notice: false, full: entry.full.clone(), kind: entry.kind, say: entry.short.clone(), detail: None, created: entry.at, asked: None };
    let d = detail(&app.cfg, &item).await;
    let mut inner = app.inner.lock().unwrap();
    if let Some(e) = inner.log.iter_mut().find(|e| e.id == id) {
        e.detail = Some(d.clone());
        inner.unsaved.borrow_mut().insert(id.to_string());
        app.save(&inner);
    }
    drop(inner);
    Some(d)
}
