//! Golden tests: what an adapter makes of a tool's real-shaped notifications and records, written down once
//! and held to, so a change to the code tools share changes nothing a tool's agents report.
//!
//! A tool's fixtures are a list of steps (`fixtures/hooks.json` in its folder), each one of:
//! - `{"hook": …}`: a notification as the tool sends it; what it reports, and what is asked of him;
//! - `{"file": {"path": …, "text" | "json": …}}`: a file written under the test's own folder first;
//! - `{"append": {"path": …, "line": …}}`: a line added to such a file;
//! - `{"env": {…}}`: the variables the hook runs with from then on (`{}` for none).
//!
//! `$D` in a step is the test's own folder. Paths under it are written down as `<tmp>`, with `/` between their
//! parts on every system. What was made is held to the golden file as JSON, never as text: how the file's lines
//! end on a checkout is not what an adapter made.
use airband_core::agent::AgentEvent;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A throwaway folder for one test, with the home it runs in (`home/` inside it). Gone when dropped.
pub(crate) struct Scratch {
    pub(crate) dir: PathBuf,
}

impl Scratch {
    pub(crate) fn new(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("airband-golden-{name}-{}", crate::config::random_hex(6)));
        std::fs::create_dir_all(dir.join("home")).unwrap();
        crate::config::tests_home::set(dir.join("home"));
        super::test_env::clear();
        Scratch { dir }
    }
    pub(crate) fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }
    /// A tool's fixture steps, with `$D` made this folder in every string: after they are read, so a folder
    /// whose path has `\` in it is never read as JSON.
    pub(crate) fn steps(&self, text: &str) -> Vec<Value> {
        let mut steps: Value = serde_json::from_str(text).expect("the fixture steps read");
        let dir = self.dir.display().to_string();
        strings(&mut steps, &|s| s.replace("$D", &dir));
        serde_json::from_value(steps).expect("the fixture is a list of steps")
    }
    /// A string as it is held to: each path under this folder written from `<tmp>`, its parts after that
    /// joined by `/` as far as the path goes (to a space or a closing bracket, where a sentence names it).
    fn written_down(&self, s: &str) -> String {
        let dir = self.dir.display().to_string();
        let (mut out, mut rest) = (String::new(), s);
        while let Some(at) = rest.find(&dir) {
            out.push_str(&rest[..at]);
            out.push_str("<tmp>");
            rest = &rest[at + dir.len()..];
            let end = rest.find(|c: char| c.is_whitespace() || c == ')').unwrap_or(rest.len());
            out.push_str(&rest[..end].replace(std::path::MAIN_SEPARATOR, "/"));
            rest = &rest[end..];
        }
        out.push_str(rest);
        out
    }
    /// Run the steps that are not notifications; give back the notification of one that is.
    pub(crate) fn step<'a>(&self, step: &'a Value) -> Option<&'a Value> {
        if let Some(file) = step.get("file") {
            let path = self.path(file["path"].as_str().unwrap());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let body = file["text"].as_str().map(String::from).unwrap_or_else(|| file["json"].to_string());
            std::fs::write(&path, body).unwrap();
            if let Some(at) = file["written"].as_u64() {
                std::fs::File::options().write(true).open(&path).unwrap().set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(at)).unwrap();
            }
        } else if let Some(more) = step.get("append") {
            use std::io::Write;
            let mut f = std::fs::File::options().append(true).open(self.path(more["path"].as_str().unwrap())).unwrap();
            writeln!(f, "{}", more["line"]).unwrap();
        } else if let Some(vars) = step.get("env") {
            super::test_env::clear();
            for (k, v) in vars.as_object().unwrap() {
                super::test_env::set(k, v.as_str().unwrap());
            }
        }
        step.get("hook")
    }
    /// Hold what was made to the golden file `golden` (its text). On a difference, what was made is written
    /// beside the system's temporary files and named, to compare.
    pub(crate) fn hold(&self, name: &str, made: &Value, golden: &str) {
        let mut made = made.clone();
        strings(&mut made, &|s| self.written_down(s));
        let golden: Value = serde_json::from_str(golden).expect("the golden file reads");
        if made != golden {
            let (text, was) = (serde_json::to_string_pretty(&made).unwrap(), serde_json::to_string_pretty(&golden).unwrap());
            let out = std::env::temp_dir().join(format!("airband-golden-{name}.json"));
            std::fs::write(&out, &text).unwrap();
            let first = text.lines().zip(was.lines()).position(|(a, b)| a != b).unwrap_or(text.lines().count().min(was.lines().count()));
            panic!(
                "{name}: what the adapter made is not what it made before; it is in {}. From line {}, made:\n{}\nbefore:\n{}",
                out.display(),
                first + 1,
                text.lines().skip(first).take(6).collect::<Vec<_>>().join("\n"),
                was.lines().skip(first).take(6).collect::<Vec<_>>().join("\n")
            );
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        super::test_env::clear();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Every string in `v`, as `change` makes it.
fn strings(v: &mut Value, change: &impl Fn(&str) -> String) {
    match v {
        Value::String(s) => *s = change(s),
        Value::Array(items) => items.iter_mut().for_each(|v| strings(v, change)),
        Value::Object(fields) => fields.values_mut().for_each(|v| strings(v, change)),
        _ => {}
    }
}

/// An event as it is held to: everything but its id, which is new every time.
pub(crate) fn event(e: &AgentEvent) -> Value {
    let mut v = serde_json::to_value(e).unwrap();
    v.as_object_mut().unwrap().remove("id");
    v
}

/// A hooks file as it now stands, or null when there is none.
pub(crate) fn file_at(path: &Path) -> Value {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

/// What a tool's adapter makes of each of its fixture steps: the events each notification reports, and for one
/// that asks leave, the request put to him and the answers the tool is given for yes, for no with a reason and
/// for a bare no.
pub(crate) fn reported(scratch: &Scratch, tool: &str, fixture: &[Value]) -> Value {
    let adapter = super::harness(tool).expect("the tool has an adapter");
    let mut made = Vec::new();
    for step in fixture {
        let Some(hook) = scratch.step(step) else { continue };
        let events: Vec<Value> = adapter.events(hook).iter().map(event).collect();
        let ask = adapter.permission(hook);
        let answers = ask.as_ref().map(|ask| serde_json::json!({ "allow": adapter.permission_answer(ask, true, ""), "deny": adapter.permission_answer(ask, false, "use staging"), "deny_bare": adapter.permission_answer(ask, false, "") }));
        made.push(serde_json::json!({ "events": events, "ask": ask, "answers": answers }));
    }
    Value::Array(made)
}

/// Its hooks put in place, twice, and taken out again: the file each time, and what he is told.
pub(crate) fn hooks_put(tool: &str) -> Value {
    let adapter = super::harness(tool).expect("the tool has an adapter");
    let Some(file) = adapter.hooks() else { return Value::Null };
    let exe = Path::new("/x/airband");
    let first = file.install(exe).unwrap();
    let after_first = file_at(&file.path);
    let second = file.install(exe).unwrap();
    let told = (adapter.installed(&file, first), adapter.installed(&file, second));
    file.uninstall(exe).unwrap();
    serde_json::json!({ "path": file.path, "changed": [first, second], "file": after_first, "told": [told.0, told.1], "after_uninstall": file_at(&file.path) })
}

/// A look at an agent, as it is held to.
pub(crate) fn seen(g: &super::Glance) -> Value {
    serde_json::json!({ "doing": g.doing, "model": g.model, "effort": g.effort })
}

/// What it has worked in, by folder, oldest written first.
pub(crate) fn folders(mut w: Vec<super::Worked>) -> Value {
    w.sort_by(|a, b| a.folder.cmp(&b.folder));
    w.iter().map(|w| serde_json::json!({ "folder": w.folder, "at": w.at })).collect()
}
