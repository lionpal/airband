//! Asking the model that reads for the tower: by its service, a command, or a server on this computer. Which
//! one is chosen, and what came of the last time, is `chosen`'s.
use super::*;

/// What a model is asked to see that it answers at all.
pub(crate) const READY: &str = "Reply with the one word: ready";

/// One question to a model behind an API key, off the async threads. `read` finds the answer in what came back.
/// One call to a service that speaks OpenAI's chat interface. `auth` is the header its key goes in and the
/// value there, as its row says; a service with no key (one on his own computer) is sent none.
pub(super) async fn ask_api(url: String, auth: Option<(String, String)>, body: Value, check: Option<airband_core::providers::Check>) -> Result<String, Trouble> {
    tokio::task::spawn_blocking(move || {
        let mut req = airband_io::http::agent(Duration::from_secs(60)).post(&url);
        if let Some((header, value)) = auth {
            req = req.header(header, value);
        }
        // What a service writes back when it refuses is never passed on or written down: some quote part of
        // the key they were sent. The status says enough.
        let mut res = req.send_json(&body).map_err(|_| Trouble::Unreachable)?;
        let status = res.status().as_u16();
        if status != 200 {
            return Err(Trouble::of_status(status, check.as_ref()));
        }
        let v: Value = res.body_mut().read_json().map_err(|_| Trouble::NoAnswer)?;
        v["choices"][0]["message"]["content"].as_str().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).ok_or(Trouble::NoAnswer)
    })
    .await
    .map_err(|_| Trouble::Program)?
}

pub(crate) use airband_core::providers::Trouble;

/// Ask the model that reads for the tower. When it does not answer, that is the answer: he is told why on his
/// phone and gets what the agent wrote, and no other model is asked.
pub(in crate::tower) async fn ask_model(settings: &Settings, prompt: String) -> Result<String, String> {
    ask_shaped(settings, prompt, None).await
}

/// The same, for a call that wants JSON of a known shape. A service that can be held to a schema is, so its
/// answer always parses; the others are asked in words, as before.
pub(super) async fn ask_shaped(settings: &Settings, prompt: String, shape: Option<Value>) -> Result<String, String> {
    let model = settings.reader();
    let asked = ask(&model, prompt, shape).await;
    // with nothing chosen there is nothing that failed
    if !model.is_none() {
        settings.asked(asked.as_ref().err().map(|failed| failed.0));
    }
    asked.map_err(|Failed(trouble, more)| if more.is_empty() { format!("{trouble:?}") } else { format!("{trouble:?} ({more})") })
}

/// What comes back from asking: the model's answer, or why there is none.
pub(super) type Asked<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, Failed>> + Send + 'a>>;

/// One way of reaching a model that reads: its name in a provider's row (`by.interface`), what it can do
/// that a row may say it needs, and what a row has to tell it.
pub(crate) struct Reader {
    pub(crate) is: Interface,
    /// It starts a program on this computer. Such an offer is honoured only as it is compiled in
    /// (`airband_core::catalog::BUILT_IN_ONLY`).
    pub(crate) runs: bool,
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

pub(super) const ON_THIS_COMPUTER: &str = "on-this-computer";

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
pub(super) async fn ask(model: &Summarizer, prompt: String, shape: Option<Value>) -> Result<String, Failed> {
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

pub(super) async fn chat(model: &Summarizer, by: &By, prompt: String, shape: Option<Value>) -> Result<String, Failed> {
    let mut body = json!({ "model": model.get(Job::Summary, "model"), "messages": [{ "role": "user", "content": prompt }] });
    if by.flag("cold") {
        body["temperature"] = json!(0);
    }
    if let (true, Some(shape)) = (by.flag("shaped"), shape) {
        body["response_format"] = shaped(shape);
    }
    let url = format!("{}/chat/completions", model.get(Job::Summary, "url").trim_end_matches('/'));
    // how this service takes a key is its row's to say; one sent no key needs nothing said
    let auth = by.key_header(&model.key).map_err(|_| Failed(Trouble::NotSet, "this version of airband was not told how that service takes its key".into()))?;
    // how its statuses read is its row's to say too
    let check = airband_core::catalog::current().providers.iter().find(|p| p.id == model.kind).and_then(|p| p.check.clone());
    Ok(ask_api(url, auth, body, check).await?)
}

pub(super) async fn on_this_computer(_model: &Summarizer, by: &By, prompt: String, shape: Option<Value>) -> Result<String, Failed> {
    let (name, server) = (by.text("model").to_string(), by.text("server").to_string());
    let url = {
        let name = name.clone();
        tokio::task::spawn_blocking(move || super::local::ensure(&name, &server, false)).await.map_err(|e| Failed(Trouble::Program, e.to_string()))?.map_err(|e| Failed(Trouble::Program, e))?
    };
    let mut body = json!({ "model": name, "messages": [{ "role": "user", "content": prompt }], "temperature": 0, "max_tokens": 400 });
    if let Some(shape) = shape {
        body["response_format"] = shaped(shape);
    }
    Ok(ask_api(format!("{url}/chat/completions"), None, body, None).await?)
}

/// The answer's shape, as a chat completions request asks for it.
fn shaped(shape: Value) -> Value {
    json!({ "type": "json_schema", "json_schema": { "name": "answer", "strict": true, "schema": shape } })
}

pub(super) async fn command(model: &Summarizer, by: &By, prompt: String, shape: Option<Value>) -> Result<String, Failed> {
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
    // `{with_shape}` stands where the row's `with_shape` arguments go when the answer must take a shape: one
    // of them holds `{shape}`, a file with that shape. A call that wants no shape has none of them.
    if let Some(at) = argv.iter().position(|a| a == "{with_shape}") {
        argv.splice(at..=at, if shape.is_some() { by.list("with_shape") } else { Vec::new() });
    }
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
