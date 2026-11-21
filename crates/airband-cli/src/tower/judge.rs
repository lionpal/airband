//! The first half of the dispatcher: how much of his attention a message deserves, and what to tell him of it.
//! Also the fuller version, for "say more". The model is whatever command the configuration names.
use super::*;
use crate::providers::{By, Interface, Job, Offer};

/// One question to a model behind an API key, off the async threads. `read` finds the answer in what came back.
/// One call to a service that speaks OpenAI's chat interface. `auth` is the header its key goes in and the
/// value there, as its row says; a service with no key (one on his own computer) is sent none.
async fn ask_api(url: String, auth: Option<(String, String)>, body: Value) -> Result<String, Trouble> {
    tokio::task::spawn_blocking(move || {
        let mut req = airband_radio::net::agent(Duration::from_secs(60)).post(&url);
        if let Some((header, value)) = auth {
            req = req.header(header, value);
        }
        // What a service writes back when it refuses is never passed on or written down: some quote part of
        // the key they were sent. The status says enough.
        let mut res = req.send_json(&body).map_err(|_| Trouble::Unreachable)?;
        let status = res.status().as_u16();
        if status != 200 {
            return Err(Trouble::of_status(status));
        }
        let v: Value = res.body_mut().read_json().map_err(|_| Trouble::NoAnswer)?;
        v["choices"][0]["message"]["content"].as_str().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).ok_or(Trouble::NoAnswer)
    })
    .await
    .map_err(|_| Trouble::Program)?
}

pub(crate) use airband_core::providers::Trouble;

/// A model that did not answer: why, and anything more for this computer's own log. The second part is never
/// sent to a phone.
pub(crate) struct Failed(pub Trouble, pub String);

impl From<Trouble> for Failed {
    fn from(trouble: Trouble) -> Failed {
        Failed(trouble, String::new())
    }
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
    use crate::providers::{Job, Use};
    // one of this tower's own list, named by its provider and its own id
    let (p, offer) = crate::providers::in_order(Job::Summary).into_iter().find(|(p, o)| p.id == wanted.kind && o.id == wanted.offer).ok_or("That is not something that can be chosen to read here.")?;
    let mut cfg = config::read().filter(|_| config::keyless().is_none()).ok_or("This computer's settings cannot be read right now.")?;
    let key = match (p.key, wanted.key.trim()) {
        (false, _) => String::new(),
        (true, "") => cfg.key_of(&p.id).ok_or_else(|| format!("{} needs a key.", p.name))?,
        (true, given) => given.to_string(),
    };
    // only the settings its row asks for
    let set = wanted.set.into_iter().filter(|(field, value)| !value.trim().is_empty() && offer.asks.iter().any(|a| a.field == *field)).map(|(f, v)| (f, Value::String(v.trim().to_string()))).collect();
    let using = Use { kind: p.id.clone(), offer: offer.id.clone(), key: key.clone(), set };
    // a model on this computer is fetched the first time it is chosen: wait through that
    super::local::wait_for_it(true);
    let tried = ask(&using, "Reply with the one word: ready".into(), None).await;
    super::local::wait_for_it(false);
    tried.map_err(|Failed(trouble, more)| {
        if !more.is_empty() {
            log(format!("trying {} to read: {more}", p.name));
        }
        trouble.said(&offer.label)
    })?;
    if p.key {
        // a provider has one key here: a new one takes the place of the one before
        cfg.keep_key(&p.id, key);
    }
    cfg.summarizer = using.clone();
    config::save(&cfg)?;
    // a model this tower was running for the last choice is stopped, unless it is the new one too
    if local_model(&using).is_none() {
        super::local::stop();
    }
    *READER.lock().unwrap() = Some(using);
    log(format!("a radio chose what reads: {}", offer.label));
    Ok(offer.label.to_string())
}

/// Why what reads could not, the last time it was asked: said to him on his phone, and gone when it next
/// answers. Nothing else is asked in its place.
static FAILED: Mutex<Option<Trouble>> = Mutex::new(None);

/// Why what reads could not, the last time it was asked. Nothing while it works.
pub(super) fn failed() -> Option<Trouble> {
    *FAILED.lock().unwrap()
}

/// Ask the model that reads for the tower. When it does not answer, that is the answer: he is told why on his
/// phone and gets what the agent wrote, and no other model is asked.
pub(super) async fn ask_model(cfg: &Config, prompt: String) -> Result<String, String> {
    ask_shaped(cfg, prompt, None).await
}

/// The same, for a call that wants JSON of a known shape. A service that can be held to a schema is, so its
/// answer always parses; the others are asked in words, as before.
pub(super) async fn ask_shaped(cfg: &Config, prompt: String, shape: Option<Value>) -> Result<String, String> {
    let model = reader(cfg);
    let asked = ask(&model, prompt, shape).await;
    // with nothing chosen there is nothing that failed
    if !model.is_none() {
        *FAILED.lock().unwrap() = asked.as_ref().err().map(|failed| failed.0);
    }
    asked.map_err(|Failed(trouble, more)| if more.is_empty() { format!("{trouble:?}") } else { format!("{trouble:?} ({more})") })
}

/// What comes back from asking: the model's answer, or why there is none.
type Asked<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, Failed>> + Send + 'a>>;

/// One way of reaching a model that reads: its name in a provider's row (`by.interface`), what it can do
/// that a row may say it needs, and what a row has to tell it.
pub(crate) struct Reader {
    pub is: Interface,
    /// It starts a program on this computer. Such an offer is honoured only as it is compiled in
    /// (`airband_core::catalog::BUILT_IN_ONLY`).
    pub runs: bool,
    ask: for<'a> fn(&'a Summarizer, &'a By, String, Option<Value>) -> Asked<'a>,
}

/// Every interface the tower reads through. A new one is a function here and a line of this table.
pub(crate) const READERS: &[Reader] = &[
    // OpenAI's Chat Completions, which nearly every model service and every local model server accepts.
    // `cold` sends `temperature: 0`, which most services take and some refuse. `shaped`: the service can be
    // held to a JSON schema, so what comes back always parses.
    Reader { is: Interface { name: "chat", can: &[], told: &[] }, runs: false, ask: |m, by, prompt, shape| Box::pin(chat(m, by, prompt, shape)) },
    // A model the tower fetches and runs itself on this computer (`local.rs`), spoken to by the same chat
    // interface held to shape. `model` is named as the server takes it; `server` is the program that runs it.
    Reader { is: Interface { name: ON_THIS_COMPUTER, can: &[], told: &["model", "server"] }, runs: true, ask: |m, by, prompt, shape| Box::pin(on_this_computer(m, by, prompt, shape)) },
    // A program on this computer: the prompt on standard input, the answer on standard output. `argv` is the
    // usual command; `argv` in his settings replaces it. It can hand the program a brief (`{brief}`, and a
    // folder to run in that holds one), the shape of the answer as a file (`{shape}`), and his settings
    // (`{model}` and the like).
    Reader { is: Interface { name: "command", can: &["brief", "shape", "settings"], told: &["argv"] }, runs: true, ask: |m, by, prompt, shape| Box::pin(command(m, by, prompt, shape)) },
];

const ON_THIS_COMPUTER: &str = "on-this-computer";

/// Does this tower have the code to read through what this offer names?
pub(crate) fn reads(offer: &Offer) -> bool {
    offer.job == Job::Summary && READERS.iter().any(|r| r.is.fits(offer))
}

/// The model this setting has the tower run on this computer, and the program that runs it, when it is one
/// of those.
pub(crate) fn local_model(model: &Summarizer) -> Option<(String, String)> {
    let (_, offer) = model.offer(Job::Summary)?;
    READERS.iter().find(|r| r.is.name == ON_THIS_COMPUTER && r.is.fits(&offer)).map(|_| (offer.by.text("model").to_string(), offer.by.text("server").to_string()))
}

/// Ask one model, through whichever interface its row names.
async fn ask(model: &Summarizer, prompt: String, shape: Option<Value>) -> Result<String, Failed> {
    if model.is_none() {
        return Err(Trouble::NotSet.into());
    }
    let (_, offer) = model.offer(Job::Summary).ok_or_else(|| Failed(Trouble::NotSet, format!("this version of airband has nothing called \"{} {}\" that reads", model.kind, model.offer)))?;
    let reader = READERS.iter().find(|r| r.is.fits(&offer)).ok_or_else(|| Failed(Trouble::NotSet, format!("this version of airband cannot read through \"{}\" yet", model.kind)))?;
    // What starts a program on this computer is run only as this build has it compiled in, whatever a
    // catalogue taken since may say (taking one already refuses such a change; this is the same rule where
    // the program is started).
    if reader.runs && !airband_core::catalog::built().providers.iter().any(|p| p.id == model.kind && p.offers.iter().any(|o| o.id == offer.id && o.job == Job::Summary && o.by == offer.by)) {
        return Err(Failed(Trouble::NotSet, format!("\"{}\" would run a program this version of airband does not come with", model.kind)));
    }
    (reader.ask)(model, &offer.by, prompt, shape).await
}

async fn chat(model: &Summarizer, by: &By, prompt: String, shape: Option<Value>) -> Result<String, Failed> {
    let mut body = json!({ "model": model.get(Job::Summary, "model"), "messages": [{ "role": "user", "content": prompt }] });
    if by.flag("cold") {
        body["temperature"] = json!(0);
    }
    if let (true, Some(shape)) = (by.flag("shaped"), shape) {
        body["response_format"] = json!({ "type": "json_schema", "json_schema": { "name": "answer", "strict": true, "schema": shape } });
    }
    let url = format!("{}/chat/completions", model.get(Job::Summary, "url").trim_end_matches('/'));
    // how this service takes a key is its row's to say; one sent no key needs nothing said
    let auth = match model.key.is_empty() {
        true => None,
        false => Some(by.auth().map(|a| (a.header.clone(), a.value(&model.key))).ok_or_else(|| Failed(Trouble::NotSet, "this version of airband was not told how that service takes its key".into()))?),
    };
    Ok(ask_api(url, auth, body).await?)
}

async fn on_this_computer(_model: &Summarizer, by: &By, prompt: String, shape: Option<Value>) -> Result<String, Failed> {
    let (name, server) = (by.text("model").to_string(), by.text("server").to_string());
    let url = {
        let name = name.clone();
        tokio::task::spawn_blocking(move || super::local::ensure(&name, &server)).await.map_err(|e| Failed(Trouble::Program, e.to_string()))?.map_err(|e| Failed(Trouble::Program, e))?
    };
    let mut body = json!({ "model": name, "messages": [{ "role": "user", "content": prompt }], "temperature": 0, "max_tokens": 400 });
    if let Some(shape) = shape {
        body["response_format"] = json!({ "type": "json_schema", "json_schema": { "name": "answer", "strict": true, "schema": shape } });
    }
    Ok(ask_api(format!("{url}/chat/completions"), None, body).await?)
}

async fn command(model: &Summarizer, by: &By, prompt: String, shape: Option<Value>) -> Result<String, Failed> {
    let program_failed = |more: String| Failed(Trouble::Program, more);
    let argv: Vec<String> = match model.set.get("argv").and_then(|a| a.as_array()) {
        Some(own) => own.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        None => by.list("argv"),
    };
    let mut shape_file: Option<std::path::PathBuf> = None;
    // `{brief}` in an argument is the file that tells a coding tool it is only rewriting a message here
    let folder = brief_folder(by).map_err(program_failed)?;
    // and `{model}`, or any other setting the row asks for, is what he set or the row's usual
    let asked: Vec<(String, String)> = model.offer(Job::Summary).map(|(_, o)| o.asks.iter().map(|a| (format!("{{{}}}", a.field), model.get(Job::Summary, &a.field))).collect()).unwrap_or_default();
    let mut argv: Vec<String> = argv.into_iter().map(|a| asked.iter().fold(a.replace("{brief}", &folder.join("brief.md").display().to_string()), |a, (name, value)| a.replace(name, value))).collect();
    // `{shape}` is a file holding the shape the answer must take, for a tool that can be held to one. A call
    // that wants no shape leaves that argument out, and the flag before it.
    if let Some(at) = argv.iter().position(|a| a.contains("{shape}")) {
        match &shape {
            Some(shape) => {
                let file = folder.join(format!("shape-{}.json", config::random_hex(6)));
                std::fs::write(&file, shape.to_string()).map_err(|e| program_failed(e.to_string()))?;
                argv[at] = argv[at].replace("{shape}", &file.display().to_string());
                shape_file = Some(file);
            }
            None => {
                argv.remove(at);
                if at > 0 && argv[at - 1].starts_with('-') {
                    argv.remove(at - 1);
                }
            }
        }
    }
    let (program, args) = argv.split_first().ok_or_else(|| Failed(Trouble::NotSet, "the command that reads is empty".into()))?;
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
        // run in the brief's folder: a tool that looks for an agent of its own to be finds the brief there
        .current_dir(&folder)
        // some tools take the folder they are in from this, not from where they were started
        .env("PWD", &folder)
        .env("AIRBAND_INTERNAL", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| program_failed(format!("cannot start {program}: {e}")))?;
    let mut stdin = child.stdin.take().unwrap();
    tokio::spawn(async move {
        let _ = stdin.write_all(prompt.as_bytes()).await;
    });
    let out = tokio::time::timeout(Duration::from_secs(90), child.wait_with_output()).await;
    if let Some(file) = shape_file {
        let _ = std::fs::remove_file(file);
    }
    let out = out.map_err(|_| Failed(Trouble::TooSlow, format!("{program} took over 90 seconds")))?.map_err(|e| program_failed(e.to_string()))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || text.is_empty() {
        return Err(program_failed(format!("{program} stopped with {}", out.status)));
    }
    Ok(text)
}

/// What a coding tool is told when it is used only to rewrite a message: one line, and no tools. Kept in a
/// folder of its own: as a plain file a tool can be pointed at, and in whatever other form a tool's row says
/// it looks for one (`brief` in the row). A general coding agent brings thousands of words of its
/// own instructions to every call; this is what keeps a summary small and quick.
const BRIEF: &str = "You rewrite messages. Reply with the JSON asked for and nothing else.";

fn brief_folder(by: &By) -> Result<std::path::PathBuf, String> {
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
pub(super) fn offered(cfg: &Config) -> airband_core::radio::ReadsOffered {
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
            free: o.local(),
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
         And \"as_written\": true only when the whole message is already one or two short plain sentences that \
         can be shown to him exactly as the agent wrote them, with nothing lost and nothing to cut: no list, no \
         code, no file path, no table. Then \"say\" is those words. Otherwise false.\n\n\
         Reply with one line of JSON and nothing else: \
         {{\"kind\":\"needs-you\" or \"report\" or \"stalled\" or \"fyi\",\"say\":\"...\",\"options\":[],\"recommended\":null,\"read\":null,\"background\":false,\"own_session\":false,\"as_written\":false}}\n\
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
        #[serde(default)]
        as_written: bool,
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
            "own_session": { "type": "boolean" },
            "as_written": { "type": "boolean" }
        },
        "required": ["say", "kind", "options", "recommended", "read", "background", "own_session", "as_written"],
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
         Reply with one line of JSON and nothing else: {{\"tell\":true or false,\"say\":\"…\"}}\n\
         \"tell\" is false when nothing in them is, and \"say\" is then empty. Otherwise \"say\" is one sentence of at most 24 words, \
         in the first person as the agent, saying what it is doing or \
         has set going, with any number or name from the labels that finds it (a build number, a commit id, where a deploy goes). \
         Do not say that anything succeeded, finished or failed: the labels do not tell. No file paths, no markdown.\n\n<steps>\n{steps}\n</steps>",
        name = cfg.name,
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
    let out = ask_shaped(cfg, prompt, Some(shape)).await.ok()?;
    let out: Out = serde_json::from_str(json_in(&out)?).ok()?;
    let said = out.say.trim();
    (out.tell && !said.is_empty() && said.chars().count() < 400).then(|| said.to_string())
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
    let item = Item { id: entry.id.clone(), notice: false, full: entry.full.clone(), kind: entry.kind, say: entry.short.clone(), asked: None };
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

#[cfg(test)]
mod tests {
    #[test]
    fn why_a_model_did_not_answer_is_told_from_the_status_and_never_in_the_services_words() {
        assert_eq!((Trouble::of_status(401), Trouble::of_status(403)), (Trouble::WrongKey, Trouble::WrongKey));
        assert_eq!((Trouble::of_status(402), Trouble::of_status(429)), (Trouble::Refused, Trouble::Refused));
        assert_eq!((Trouble::of_status(404), Trouble::of_status(503)), (Trouble::NotFound, Trouble::Unexpected { status: 503 }));
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
