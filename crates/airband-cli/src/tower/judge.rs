//! The first half of the dispatcher: how much of his attention a message deserves, and what to tell him of it.
//! Also the fuller version, for "say more". The model is whatever command the configuration names.
use super::*;

/// One question to a model behind an API key, off the async threads. `read` finds the answer in what came back.
/// One call to a service that speaks OpenAI's chat interface. A service with no key (one on his own computer)
/// is sent none.
async fn ask_api(url: String, key: String, body: Value) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let mut req = airband_radio::net::agent(Duration::from_secs(60)).post(&url);
        if !key.is_empty() {
            req = req.header("authorization", format!("Bearer {key}"));
        }
        let mut res = req.send_json(&body).map_err(|e| format!("the model could not be reached: {e}"))?;
        let status = res.status().as_u16();
        let v: Value = res.body_mut().read_json().map_err(|e| e.to_string())?;
        if status != 200 {
            return Err(format!("the model refused ({status}): {}", v["error"]["message"].as_str().unwrap_or("no reason given").chars().take(160).collect::<String>()));
        }
        v["choices"][0]["message"]["content"].as_str().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).ok_or_else(|| "the model answered with nothing".to_string())
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

/// What reads, when he has changed it from a radio since this tower started: used from then on, with no
/// restart. The settings file has it too, for the next start.
static READER: Mutex<Option<Summarizer>> = Mutex::new(None);

/// What reads for the tower now.
pub(super) fn reader(cfg: &Config) -> Summarizer {
    READER.lock().unwrap().clone().unwrap_or_else(|| cfg.summarizer.clone())
}

/// Choose what reads, asked for from a radio. Only a provider he could choose in `airband setup`, reached the
/// way its row says: a radio cannot name a command to run here. It is tried with a real call before anything
/// is kept. The key is kept encrypted with the others. Returns the provider's name.
pub(super) async fn set_reader(wanted: airband_core::radio::SetReads) -> Result<String, String> {
    use crate::providers::{Job, Use, offering};
    let p = offering(Job::Summary).find(|p| p.id == wanted.kind).ok_or("That is not something that can be chosen to read here.")?;
    let offer = p.offers.iter().find(|o| o.job == Job::Summary).ok_or("That does not read.")?;
    let mut cfg = config::read().filter(|_| config::keyless().is_none()).ok_or("This computer's settings cannot be read right now.")?;
    let key = match (p.key, wanted.key.trim()) {
        (false, _) => String::new(),
        (true, "") => cfg.keys.values().find(|k| k.provider == p.id).map(|k| k.secret.clone()).ok_or_else(|| format!("{} needs a key.", p.name))?,
        (true, given) => given.to_string(),
    };
    // only the settings its row asks for
    let set = wanted.set.into_iter().filter(|(field, value)| !value.trim().is_empty() && offer.asks.iter().any(|a| a.field == field)).map(|(f, v)| (f, Value::String(v.trim().to_string()))).collect();
    let using = Use { kind: p.id.into(), key: key.clone(), set };
    // a model on this computer is fetched the first time it is chosen: wait through that
    super::local::wait_for_it(true);
    let tried = ask(&using, "Reply with the one word: ready".into(), None).await;
    super::local::wait_for_it(false);
    tried.map_err(|why| format!("{} did not answer: {}", p.name, why.chars().take(160).collect::<String>()))?;
    if p.key && !cfg.keys.values().any(|k| k.secret == key) {
        let name = (1..).map(|n| if n == 1 { p.id.to_string() } else { format!("{}-{n}", p.id) }).find(|n| !cfg.keys.contains_key(n)).unwrap();
        cfg.keys.insert(name, config::StoredKey { provider: p.id.into(), secret: key });
    }
    cfg.summarizer = using.clone();
    config::save(&cfg)?;
    // a model this tower was running for the last choice is stopped, unless it is the new one too
    if !matches!(offer.by, crate::providers::By::OnThisComputer(_)) {
        super::local::stop();
    }
    *READER.lock().unwrap() = Some(using);
    log(format!("a radio chose what reads: {}", offer.label));
    Ok(offer.label.to_string())
}

/// Ask the model that reads for the tower. If it does not answer, the second one is asked, when there is one.
pub(super) async fn ask_model(cfg: &Config, prompt: String) -> Result<String, String> {
    ask_shaped(cfg, prompt, None).await
}

/// The same, for a call that wants JSON of a known shape. A service that can be held to a schema is, so its
/// answer always parses; the others are asked in words, as before.
pub(super) async fn ask_shaped(cfg: &Config, prompt: String, shape: Option<Value>) -> Result<String, String> {
    match ask(&reader(cfg), prompt.clone(), shape.clone()).await {
        Ok(text) => Ok(text),
        Err(first) => match &cfg.summarizer_fallback {
            Some(second) => {
                log(format!("the model did not answer ({first}); asking the second"));
                ask(second, prompt, shape).await.map_err(|e| format!("{first}; and the second: {e}"))
            }
            None => Err(first),
        },
    }
}

/// Ask one model, through whichever interface its row names.
async fn ask(model: &Summarizer, prompt: String, shape: Option<Value>) -> Result<String, String> {
    use crate::providers::{By, Job};
    if model.is_none() {
        return Err("no summarizer is configured".into());
    }
    let (_, offer) = model.offer(Job::Summary).ok_or_else(|| format!("this version of airband has nothing called \"{}\" that reads", model.kind))?;
    let argv: Vec<String> = match offer.by {
        // OpenAI's chat interface, which the row's service speaks at its address
        By::Chat { cold, shaped } => {
            let mut body = json!({ "model": model.get(Job::Summary, "model"), "messages": [{ "role": "user", "content": prompt }] });
            if cold {
                body["temperature"] = json!(0);
            }
            if let (true, Some(shape)) = (shaped, shape) {
                body["response_format"] = json!({ "type": "json_schema", "json_schema": { "name": "answer", "strict": true, "schema": shape } });
            }
            let url = format!("{}/chat/completions", model.get(Job::Summary, "url").trim_end_matches('/'));
            return ask_api(url, model.key.clone(), body).await;
        }
        // a model this tower runs itself: the same chat interface, at its own address, always held to shape
        By::OnThisComputer(name) => {
            let url = tokio::task::spawn_blocking(move || super::local::ensure(name)).await.map_err(|e| e.to_string())??;
            let mut body = json!({ "model": name, "messages": [{ "role": "user", "content": prompt }], "temperature": 0, "max_tokens": 400 });
            if let Some(shape) = shape {
                body["response_format"] = json!({ "type": "json_schema", "json_schema": { "name": "answer", "strict": true, "schema": shape } });
            }
            return ask_api(format!("{url}/chat/completions"), String::new(), body).await;
        }
        By::Command(usual) => match model.set.get("argv").and_then(|a| a.as_array()) {
            Some(own) => own.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
            None => usual.iter().map(|s| s.to_string()).collect(),
        },
        _ => return Err(format!("\"{}\" does not read", model.kind)),
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
    /// The agent says it will take this answer only from him in its own session, and not from a message that
    /// reaches it another way: an answer sent from a radio to an agent the tower does not run would not count.
    pub own_session: bool,
}

/// The tower's main job: decide how much of his attention a message deserves, what to tell him of it, and, when
/// it is a question he can answer by picking, what the picks are.
pub(super) async fn prepare(cfg: &Config, item: &Item) -> Judged {
    if item.notice {
        return Judged { kind: Kind::NeedsYou, say: spoken(&item.full), options: Vec::new(), recommended: None, read: None, background: false, own_session: false };
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
         - Always keep anything the work is waiting on that only he can do (a request he has to file, an approval, \
         a payment, a sign-in, a setting only his account can change, a choice he has not made), whether or not \
         the agent has stopped for it: an agent that went on with other work has still left that part blocked on \
         him. Say it in its own sentence, beginning \"Waiting on you:\", with what he has to do and what it \
         unblocks (\"Waiting on you: request the CarPlay entitlement from Apple, before the car screen can ship.\"). \
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
         Reply with one line of JSON and nothing else: \
         {{\"kind\":\"needs-you\" or \"report\" or \"stalled\" or \"fyi\",\"say\":\"...\",\"options\":[],\"recommended\":null,\"read\":null,\"background\":false,\"own_session\":false}}\n\
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
        #[serde(default)]
        own_session: bool,
    }
    // The shape of the answer, for a service that can be held to one. The words come before the verdict: a
    // small model that has just written what the message says judges it better than one that judges first.
    let shape = json!({
        "type": "object",
        "properties": {
            "say": { "type": "string" },
            "kind": { "type": "string", "enum": ["needs-you", "report", "stalled", "fyi"] },
            "options": { "type": "array", "items": { "type": "string" }, "maxItems": 3 },
            "recommended": { "type": ["integer", "null"] },
            "read": { "type": ["string", "null"] },
            "background": { "type": "boolean" },
            "own_session": { "type": "boolean" }
        },
        "required": ["say", "kind", "options", "recommended", "read", "background", "own_session"],
        "additionalProperties": false
    });
    let parsed = ask_shaped(cfg, prompt, Some(shape)).await.and_then(|out| {
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
            let own_session = o.own_session && kind == Kind::NeedsYou;
            let options = if kind == Kind::NeedsYou && !own_session { o.options.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty() && c.chars().count() <= 24).take(3).collect() } else { Vec::new() };
            let recommended = o.recommended.filter(|i| (*i as usize) < options.len());
            // only for a message with more in it than its line: a name of a few words, and nothing for chatter
            let read = o.read.map(|r| r.trim().to_string()).filter(|r| !r.is_empty() && r.chars().count() <= 32 && kind != Kind::Fyi && spoken(&item.full) != say);
            Judged { kind, say, options, recommended, read, background: o.background && kind != Kind::NeedsYou, own_session }
        }
        other => {
            // No model answered. Nothing is guessed from the words: he gets the message as the agent wrote it,
            // marked as a report unless its harness said what it is.
            if let Err(e) = other {
                log(format!("no summary, giving him the message as written: {e}"));
            }
            Judged { kind: item.kind.unwrap_or(Kind::Report), say: spoken(&item.full), options: Vec::new(), recommended: None, read: None, background: false, own_session: false }
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
///
/// The model sees his words only. The request is text the asking agent wrote, and nothing an agent writes is
/// put beside the one question whose answer lets a command run.
pub(super) async fn consent(cfg: &Config, said: &str) -> Option<bool> {
    let prompt = format!(
        "A coding agent asked {name} for permission to do one thing on his computer. He was shown exactly what, \
         and answered in his own words.\n\nHis answer:\n<answer>\n{said}\n</answer>\n\n\
         Reply with one word and nothing else:\n\
         - allow: he plainly says yes to exactly what was asked, with no condition and no change.\n\
         - refuse: he says no, or tells it what to do instead or first, or attaches a condition or a change \
         (\"run it on staging first\", \"not that folder\", \"only the tests\", \"yes but skip the migration\"). An \
         instruction of his own is a refusal of what was asked: it goes back to the agent as the reason.\n\
         - unclear: only when his words give the agent nothing to go on: he hands the choice back (\"whatever you \
         think\"), asks a question, or is talking about something else.\n\
         When in doubt between allow and anything else, it is not allow.",
        name = cfg.name,
        said = tail(said, 1000)
    );
    let out = ask_model(cfg, prompt).await.ok()?;
    if std::env::var_os("AIRBAND_TRACE").is_some() {
        log(format!("asked whether \"{said}\" allows it, and was told: {out}"));
    }
    // one word was asked for; a model that wraps it in JSON all the same has still said it
    let word = json_in(&out).and_then(|j| serde_json::from_str::<Value>(j).ok()).and_then(|v| v.as_object().and_then(|o| o.values().find_map(|x| x.as_str().map(String::from)))).unwrap_or(out.clone());
    match word.trim().trim_matches(|c: char| !c.is_ascii_alphabetic()).to_ascii_lowercase().as_str() {
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
