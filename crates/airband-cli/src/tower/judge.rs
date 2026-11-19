//! The first half of the dispatcher: how much of his attention a message deserves, and what to tell him of it.
//! Also the fuller version, for "say more". The model is whatever command the configuration names.
use super::*;

pub(super) async fn ask_model(cfg: &Config, prompt: String) -> Result<String, String> {
    let Summarizer::Command { argv } = &cfg.summarizer else { return Err("no summarizer is configured".into()) };
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
}

/// The tower's main job: decide how much of his attention a message deserves, what to tell him of it, and, when
/// it is a question he can answer by picking, what the picks are.
pub(super) async fn prepare(cfg: &Config, item: &Item) -> Judged {
    let asks = item.full.trim_end().ends_with('?');
    if item.notice {
        return Judged { kind: Kind::NeedsYou, say: spoken(&item.full), options: Vec::new(), recommended: None, read: None };
    }
    let prompt = format!(
        "{name} runs several coding agents at once and is away from his desk. You are his dispatcher: you decide \
         what reaches him and in how few words. Below is what one agent wrote when it stopped.\n\n\
         First, how much it deserves his attention:\n\
         - \"needs-you\": the agent is stopped and will not go on by itself: it is waiting for his answer or for \
         something only he can do, or it was halted (a usage limit, an error, a missing login). Also when \
         something broke, was lost or went wrong that he has to know now.\n\
         - \"report\": it finished a piece of work, or found or decided something he would want to know: a result, \
         a choice he might disagree with, something it left undone.\n\
         - \"stalled\": it has stopped part way. It says what it will do next (\"Next I will…\", \"I'll now…\"), \
         or that it is starting or carrying on, but this is where it stopped: the work is not finished and it \
         asks him nothing. Remember that it wrote this as it stopped, so nothing it says it is about to do is \
         being done. It needs a nudge to go on, not an answer. The exception is an agent waiting for something \
         that will bring it back by itself (a build, a test run, a background task): that is \"fyi\".\n\
         - \"fyi\": he would not miss it. Progress chatter, an acknowledgement (\"OK, starting\"), a plan it is \
         about to carry out anyway, a restatement of what he just told it, an interim note while work goes on.\n\
         When unsure between two, pick the lower one, unless the agent is waiting on him or has stopped with \
         work it said it would still do.\n\n\
         Then what to tell him, in plain spoken English. He will not read much: he glances. One short sentence, \
         14 words at most. If the agent is asking him something, the sentence is the question itself, as \
         briefly as it can be put (\"Two questions are too easy. Rewrite or leave?\"); how it got there is \
         one tap away and does not belong here.\n\
         The one exception is findings he asked for. If the last thing he told this agent was to investigate, \
         research, review, compare, audit or explain something, and this message is the answer, a headline would \
         throw the answer away: then write an executive summary instead, three to five short sentences and 90 \
         words at most, the conclusion first, then the findings that matter with their numbers, then what he \
         has to decide if anything. It is read aloud to him, so it is sentences, never a list.\n\
         - Keep only: the result; anything left undone or gone wrong; the one thing he must decide.\n\
         - If the agent needs an answer from him, end with that question, with the options if it gave any.\n\
         - No file paths, code, identifiers, lists or markdown. Say \"the checker script\", not its filename.\n\
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
         {{\"kind\":\"needs-you\" or \"report\" or \"stalled\" or \"fyi\",\"say\":\"...\",\"options\":[],\"recommended\":null,\"read\":null}}\n\n\
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
    }
    let parsed = ask_model(cfg, prompt).await.and_then(|out| {
        let (a, b) = (out.find('{').ok_or("no JSON in the answer")?, out.rfind('}').ok_or("no JSON in the answer")?);
        serde_json::from_str::<Out>(&out[a..=b]).map_err(|e| e.to_string())
    });
    // with no judge, err towards telling him
    let guess = if asks { Kind::NeedsYou } else { Kind::Report };
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
            Judged { kind, say, options, recommended, read }
        }
        other => {
            if let Err(e) = other {
                log(format!("no summary, reading the opening instead: {e}"));
            }
            Judged { kind: item.kind.unwrap_or(guess), say: opening(&item.full), options: Vec::new(), recommended: None, read: None }
        }
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
