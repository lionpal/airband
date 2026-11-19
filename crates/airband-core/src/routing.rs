//! Callsigns, and working out what an utterance means and who it is for.

const WORDS: [&str; 13] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve"];
/// What speech recognition tends to write when a number is said after a name.
const SOUNDS_LIKE: [(&str, &str); 9] =
    [("won", "one"), ("to", "two"), ("too", "two"), ("for", "four"), ("fore", "four"), ("ate", "eight"), ("tree", "three"), ("free", "three"), ("sex", "six")];

/// A session's name on the radio: its folder's name plus a number word ("shop two"), or plus the name he gave
/// the session ("warehouse removals"). `number` holds whichever follows the project's name.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Callsign {
    pub project: String,
    pub number: String,
}

impl Callsign {
    pub fn text(&self) -> String {
        format!("{} {}", self.project, self.number)
    }
    /// Capitalised, to start a sentence.
    pub fn spoken(&self) -> String {
        let t = self.text();
        let mut c = t.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
    }
    /// The callsign a session's own name asks for, in this folder: "shop 2" → shop two, "removals" or
    /// "warehouse removals" → warehouse removals. Nothing when the name says nothing beyond the project.
    pub fn titled(cwd: &str, title: &str) -> Option<Callsign> {
        let project = Callsign::next_for(cwd, &[]).project;
        let title = norm(title);
        // his name for it usually starts with the project's ("shop 2", "back office fixes"): that part is not said twice
        let words: Vec<&str> = title.split(' ').filter(|w| !w.is_empty()).collect();
        let lead = (1..=words.len()).find(|&n| words[..n].concat() == project.replace(' ', "")).unwrap_or(0);
        let rest: Vec<String> = words[lead..].iter().take(4).map(|w| number_word(w)).collect();
        (!rest.is_empty()).then(|| Callsign { project, number: rest.join(" ") })
    }
    /// The next free callsign for a session in this folder: the folder's name plus the lowest unused number.
    pub fn next_for(cwd: &str, live: &[Callsign]) -> Callsign {
        let base = cwd.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("");
        let mut project = norm(base);
        if project.is_empty() {
            project = "session".into();
        }
        let mut n = 1;
        loop {
            let number = WORDS.get(n).map(|w| w.to_string()).unwrap_or_else(|| n.to_string());
            if !live.iter().any(|c| c.project == project && c.number == number) {
                return Callsign { project, number };
            }
            n += 1;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// "say more": the fuller version of what was last said
    More,
    Repeat,
    /// who is waiting, who is working
    Status,
    /// no reply; move on
    Skip,
    /// stop sending transmissions until told to go ahead
    Hold,
    Resume,
    /// set going again every agent that has stopped without finishing
    Poke,
}

const CONTROLS: [(Control, &[&str]); 7] = [
    (Control::Poke, &["poke", "poke them", "poke everyone", "poke all", "give them a poke"]),
    (Control::More, &["say more", "more", "more detail", "more details", "details", "elaborate", "tell me more", "go on"]),
    (Control::Repeat, &["repeat", "say again", "again", "repeat that", "say that again"]),
    (Control::Status, &["status", "whos waiting", "who is waiting", "roll call", "sitrep", "whats waiting"]),
    (Control::Skip, &["skip", "next", "no reply", "roger", "copy", "copy that", "nothing", "never mind"]),
    (Control::Hold, &["stand by", "standby", "hold", "quiet", "hold traffic"]),
    (Control::Resume, &["go ahead", "resume", "continue traffic"]),
];

#[derive(Debug, PartialEq, Eq)]
pub struct Parsed {
    /// Indexes into the callsigns given: everyone this was addressed to. Empty when nobody was named.
    pub to: Vec<usize>,
    /// A tower command, when everything after the addressing is one.
    pub control: Option<Control>,
    /// The words meant for the agents, in his own wording. Empty when it was a control.
    pub rest: String,
}

impl Parsed {
    /// The one agent named, when exactly one was.
    pub fn session(&self) -> Option<usize> {
        (self.to.len() == 1).then(|| self.to[0])
    }
}

fn norm(t: &str) -> String {
    let lowered = t.to_lowercase().replace(['’', '\''], "");
    lowered.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}
fn key(word: &str) -> String {
    norm(word).replace(' ', "")
}
fn number_word(w: &str) -> String {
    let w = SOUNDS_LIKE.iter().find(|(heard, _)| *heard == w).map(|(_, said)| *said).unwrap_or(w);
    match w.parse::<usize>() {
        Ok(n) if n < WORDS.len() => WORDS[n].to_string(),
        _ => w.to_string(),
    }
}
fn digits(number: &str) -> String {
    WORDS.iter().position(|w| *w == number).map(|n| n.to_string()).unwrap_or_else(|| number.to_string())
}
/// Is what follows the project's name a name of his own ("removals"), not a number? A tag may then be that
/// name alone (`@removals`).
fn own_name(number: &str) -> bool {
    number.len() > 3 && !WORDS.contains(&number) && number.parse::<usize>().is_err()
}
const EVERYONE: [&str; 5] = ["all", "everyone", "everybody", "allstations", "allagents"];

/// Who a typed tag means: `@all`, a whole group (`@shop`), or one agent (`@shop2`, `@shop-two`).
fn tagged(tag: &str, live: &[Callsign]) -> Option<Vec<usize>> {
    let k = key(tag);
    if EVERYONE.contains(&k.as_str()) {
        return Some((0..live.len()).collect());
    }
    let hits: Vec<usize> = live
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let project = c.project.replace(' ', "");
            let number = c.number.replace(' ', "");
            k == project || k == format!("{project}{number}") || k == format!("{project}{}", digits(&c.number)) || (own_name(&c.number) && k == number)
        })
        .map(|(i, _)| i)
        .collect();
    (!hits.is_empty()).then_some(hits)
}

/// What he said or typed → who it is for and what it says.
///
/// Typed: any number of leading tags: `@shop2 @backoffice leave it`, `@shop stop` (the whole group), `@all`.
/// Spoken: a leading callsign ("shop two, leave it"), "all shop, stop", or "everyone, stop".
pub fn parse_utterance(text: &str, live: &[Callsign]) -> Parsed {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    // "Tower, status" and "Airband, status" both mean the dispatcher itself. Speech may write "air band".
    if words.first().is_some_and(|w| matches!(key(w).as_str(), "tower" | "airband")) {
        words.remove(0);
    } else if words.len() > 1 && key(words[0]) == "air" && key(words[1]) == "band" {
        words.drain(..2);
    }
    let mut to: Vec<usize> = Vec::new();
    let mut taken = 0;
    let add = |to: &mut Vec<usize>, more: Vec<usize>| more.into_iter().for_each(|i| if !to.contains(&i) { to.push(i) });

    while let Some(hit) = words.get(taken).filter(|w| w.starts_with('@')).and_then(|w| tagged(w, live)) {
        add(&mut to, hit);
        taken += 1;
    }
    if taken == 0 {
        let first = words.first().map(|w| key(w)).unwrap_or_default();
        let second = words.get(1).map(|w| key(w)).unwrap_or_default();
        let group = |name: &str| -> Vec<usize> { live.iter().enumerate().filter(|(_, c)| c.project.replace(' ', "") == name).map(|(i, _)| i).collect() };
        if first == "all" && !group(&second).is_empty() {
            // "all shop, stop"
            add(&mut to, group(&second));
            taken = 2;
        } else if matches!(first.as_str(), "everyone" | "everybody") || (first == "all" && matches!(second.as_str(), "stations" | "agents")) {
            add(&mut to, (0..live.len()).collect());
            taken = if first == "all" { 2 } else { 1 };
        } else {
            for (i, c) in live.iter().enumerate() {
                let project = c.project.replace(' ', "");
                let alone = live.iter().filter(|o| o.project == c.project).count() == 1;
                // "backoffice" may be heard as "back office": compare with the spaces taken out, over the first few words
                for n in 1..=words.len().min(5) {
                    let head: Vec<String> = words[..n].iter().map(|w| key(w)).collect();
                    let joined = head.concat();
                    let with_number = format!("{}{}", head[..n - 1].concat(), number_word(&head[n - 1]));
                    let number = c.number.replace(' ', "");
                    let hit = with_number == format!("{project}{number}") || joined == format!("{project}{number}") || (alone && joined == project);
                    // the longest match wins, so "shop two" beats a bare "shop"
                    if hit && n > taken {
                        taken = n;
                        to = vec![i];
                    }
                }
            }
        }
    }
    let rest = words[taken..].join(" ");
    let rest = rest.trim_start_matches([' ', ',', '.', ':', ';', '-']).trim().to_string();
    let said = norm(&rest);
    let control = CONTROLS.iter().find(|(_, phrases)| phrases.contains(&said.as_str())).map(|(c, _)| *c);
    Parsed { to, control, rest: if control.is_some() { String::new() } else { rest } }
}

/// Which model a new agent should run on and how hard it should think, when he says so before the task:
/// "with fable high, find out why checkout is slow".
#[derive(Debug, PartialEq, Eq)]
pub struct With {
    /// The model, as the launcher lists it.
    pub model: String,
    /// The effort, as the launcher lists it, if he gave one.
    pub effort: Option<String>,
    /// The task: everything after.
    pub rest: String,
}

impl With {
    /// With no other sign that he wants a new agent, do these words ask for one? "With fable, look into the
    /// refund failures" does. "With opus it is too slow" is a remark to whoever he is talking to, and is not.
    pub fn asks_for_an_agent(&self) -> bool {
        let mut words = self.rest.split_whitespace();
        let first = words.next().map(key).unwrap_or_default();
        const REMARK: &[&str] = &["it", "its", "is", "was", "were", "we", "the", "that", "this", "there", "being", "selected", "on", "in", "and", "or", "as"];
        words.count() >= 2 && !REMARK.contains(&first.as_str())
    }
}

/// Words that open with a model: "with fable", "using opus high", "hey, with sonnet at max effort: …". `models`
/// and `efforts` are what the launcher in use offers; a model it does not list is never matched. "Extra high"
/// is `xhigh` and "maximum" is `max` when the launcher lists those.
pub fn parse_with(text: &str, models: &[String], efforts: &[String]) -> Option<With> {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    while words.first().is_some_and(|w| matches!(key(w).as_str(), "hey" | "ok" | "okay" | "so" | "tower" | "airband")) {
        words.remove(0);
    }
    if !matches!(key(words.first()?).as_str(), "with" | "using") {
        return None;
    }
    let model = models.iter().find(|m| key(m) == key(words.get(1).copied().unwrap_or_default()))?.clone();
    let mut at = 2;
    let level = |at: usize| -> Option<(String, usize)> {
        let one = key(words.get(at).copied()?);
        let two = words.get(at + 1).map(|w| format!("{one}{}", key(w)));
        let said = |name: &str| efforts.iter().find(|e| key(e) == name).cloned();
        match (one.as_str(), two.as_deref()) {
            (_, Some("extrahigh")) => said("xhigh").map(|e| (e, 2)),
            ("maximum", _) => said("max").map(|e| (e, 1)),
            ("med", _) => said("medium").map(|e| (e, 1)),
            _ => said(&one).map(|e| (e, 1)),
        }
    };
    // "fable high", "fable on high", "fable at max effort"
    let lead = usize::from(words.get(at).is_some_and(|w| matches!(key(w).as_str(), "on" | "at" | "set")) && level(at + 1).is_some());
    let effort = level(at + lead).map(|(e, n)| {
        at += lead + n;
        e
    });
    if effort.is_some() && words.get(at).is_some_and(|w| matches!(key(w).as_str(), "effort" | "thinking" | "reasoning")) {
        at += 1;
    }
    let rest = words[at..].join(" ").trim_start_matches([' ', ',', '.', ':', ';', '-']).trim().to_string();
    Some(With { model, effort, rest })
}

/// Two characters to stand for an agent where its whole callsign will not fit: the team's initial and its
/// number, or the initial of the name he gave it. "shop three" is S3, "catalog part page" is CP.
pub fn badge(project: &str, callsign: &str) -> String {
    let initial = |w: &str| w.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    let rest = callsign.strip_prefix(project).unwrap_or(callsign).trim();
    let first = rest.split_whitespace().next().unwrap_or("");
    let second = match WORDS.iter().position(|w| *w == first.to_lowercase()) {
        Some(n) => n.to_string(),
        None => initial(first),
    };
    format!("{}{}", initial(project), second)
}

/// A request to start a new agent: which project, and what it should do.
#[derive(Debug, PartialEq, Eq)]
pub struct Launch {
    /// The project he named. Nothing when he gave only the idea ("new idea: warn the warehouse when a part is
    /// about to sell out"): the tower then works out which project it belongs in.
    pub project: Option<String>,
    /// His words for the task, untouched. Empty when he named a project and nothing else.
    pub task: String,
}

/// Is this a request to start a new agent in one of the projects the tower knows?
///
/// Typed: `/launch shop write the unit 5 questions` (also `/new`). Spoken: "launch shop: write the unit 5
/// questions", "start a new agent in shop to write the unit 5 questions", "spin up an agent for back office, …".
/// "Start" on its own is not enough ("start with the first one" is a reply), so it needs "agent", "session" or
/// "new" beside it; "launch" and "spin up" stand alone. A project that is not known is never launched into.
pub fn parse_launch(text: &str, projects: &[String]) -> Option<Launch> {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    if words.first().is_some_and(|w| matches!(key(w).as_str(), "tower" | "airband")) {
        words.remove(0);
    }
    let first = words.first()?;
    let slash = first.starts_with('/');
    let verb = key(first);
    let mut at = 1;
    let mut sure = match (slash, verb.as_str()) {
        (true, "launch" | "new" | "start") => true,
        (true, _) => return None,
        (false, "launch") => true,
        (false, "spin") if words.get(1).is_some_and(|w| key(w) == "up") => {
            at = 2;
            true
        }
        (false, "start" | "open" | "new") => verb == "new",
        _ => return None,
    };
    // "a new agent in", "an agent for", "session on": words that only lead up to the project's name
    let (mut marked, mut naming) = (false, false);
    while let Some(w) = words.get(at) {
        match key(w).as_str() {
            "new" => sure = true,
            // these say outright that he means a new agent, so a project need not follow
            "agent" | "session" | "idea" => {
                sure = true;
                marked = true;
                naming = false;
            }
            // these say a project's name comes next
            "in" | "on" | "for" => naming = true,
            "a" | "an" | "one" => {}
            _ => break,
        }
        at += 1;
    }
    if !sure {
        return None;
    }
    // the longest run of the next words that spells a known project ("back office" is backoffice)
    let mut found: Option<(usize, &String)> = None;
    for n in 1..=3.min(words.len().saturating_sub(at)) {
        let spelled: String = words[at..at + n].iter().map(|w| key(w)).collect();
        if let Some(p) = projects.iter().find(|p| p.replace(' ', "") == spelled) {
            found = Some((n, p));
        }
    }
    // No project he knows of follows. If he was naming one ("a new agent in payroll"), it is one the tower does
    // not have, and this is not a launch. If he went straight to the idea ("new idea: …", "a new agent to …"),
    // it is a launch with the project left for the tower to work out.
    let (n, project) = match found {
        Some((n, p)) => (n, Some(p.clone())),
        None if marked && !naming => (0, None),
        None => return None,
    };
    let mut rest = &words[at + n..];
    while rest.first().is_some_and(|w| matches!(key(w).as_str(), "to" | "and" | "that" | "which")) {
        rest = &rest[1..];
    }
    let task = rest.join(" ");
    Some(Launch { project, task: task.trim_start_matches([' ', ',', '.', ':', ';', '-']).trim().to_string() })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_session_takes_the_name_he_gave_it() {
        let c = |p: &str, n: &str| Callsign { project: p.into(), number: n.into() };
        assert_eq!(Callsign::titled("/x/shop", "shop 2"), Some(c("shop", "two")));
        assert_eq!(Callsign::titled("/x/warehouse", "warehouse removals"), Some(c("warehouse", "removals")));
        assert_eq!(Callsign::titled("/x/backoffice", "Back Office: login fix"), Some(c("backoffice", "login fix")));
        assert_eq!(Callsign::titled("/x/shop", "curriculum"), Some(c("shop", "curriculum")));
        assert_eq!(Callsign::titled("/x/shop", "shop"), None);
        let live = [c("warehouse", "removals"), c("warehouse", "one"), c("backoffice", "login fix")];
        assert_eq!(parse_utterance("warehouse removals, stop", &live).to, vec![0]);
        // spoken, a name of his own needs its project in front: "removals look fine" is a sentence, not an address
        assert!(parse_utterance("removals look fine", &live).to.is_empty());
        assert!(parse_utterance("@one stop", &live).to.is_empty());
        assert_eq!(parse_utterance("back office login fix, ship it", &live).to, vec![2]);
        assert_eq!(parse_utterance("@removals stop", &live).to, vec![0]);
        assert_eq!(parse_utterance("@backofficeloginfix ship", &live).to, vec![2]);
        assert_eq!(parse_utterance("@warehouse stop", &live).to, vec![0, 1]);
    }

    use super::*;

    fn live() -> Vec<Callsign> {
        [("shop", "one"), ("shop", "two"), ("backoffice", "one")].iter().map(|(p, n)| Callsign { project: p.to_string(), number: n.to_string() }).collect()
    }
    fn p(t: &str) -> (Option<usize>, Option<Control>, String) {
        let u = parse_utterance(t, &live());
        (u.session(), u.control, u.rest)
    }

    #[test]
    fn callsign_then_message() {
        assert_eq!(p("Shop two, rewrite both questions."), (Some(1), None, "rewrite both questions.".into()));
    }
    #[test]
    fn digits_and_sound_alikes() {
        assert_eq!(p("shop 2 leave them"), (Some(1), None, "leave them".into()));
        assert_eq!(p("Shop to leave them"), (Some(1), None, "leave them".into()));
        assert_eq!(p("Sh op won go"), (Some(0), None, "go".into()));
    }
    #[test]
    fn name_heard_as_two_words_and_bare_name_when_alone() {
        assert_eq!(p("Back office one ship it"), (Some(2), None, "ship it".into()));
        assert_eq!(p("back office, ship it"), (Some(2), None, "ship it".into()));
    }
    #[test]
    fn bare_name_shared_by_two_picks_neither() {
        assert_eq!(p("shop leave them"), (None, None, "shop leave them".into()));
    }
    #[test]
    fn no_callsign() {
        assert_eq!(p("yes go ahead and fix it"), (None, None, "yes go ahead and fix it".into()));
    }
    #[test]
    fn controls() {
        assert_eq!(p("say more"), (None, Some(Control::More), String::new()));
        assert_eq!(p("Tower, status."), (None, Some(Control::Status), String::new()));
        assert_eq!(p("Airband, status."), (None, Some(Control::Status), String::new()));
        assert_eq!(p("Air band, who's waiting"), (None, Some(Control::Status), String::new()));
        assert_eq!(p("who's waiting"), (None, Some(Control::Status), String::new()));
        assert_eq!(p("shop two say again"), (Some(1), Some(Control::Repeat), String::new()));
        assert_eq!(p("Stand by."), (None, Some(Control::Hold), String::new()));
    }
    #[test]
    fn control_word_inside_a_sentence_is_not_a_control() {
        assert_eq!(p("skip the second question and repeat the check").1, None);
    }
    #[test]
    fn callsign_alone() {
        assert_eq!(p("shop one"), (Some(0), None, String::new()));
    }
    #[test]
    fn typed_tags_reach_one_a_group_or_everyone() {
        let to = |t: &str| parse_utterance(t, &live()).to;
        assert_eq!(to("@shop2 leave them"), vec![1]);
        assert_eq!(to("@shop-two leave them"), vec![1]);
        assert_eq!(to("@shop stop and commit"), vec![0, 1]);
        assert_eq!(to("@shop1 @backoffice status report please"), vec![0, 2]);
        assert_eq!(to("@all stop"), vec![0, 1, 2]);
        assert_eq!(parse_utterance("@shop2 @backoffice ship it", &live()).rest, "ship it");
        // a tag for nobody is part of the message, not addressing
        assert_eq!(parse_utterance("@nobody hello", &live()).rest, "@nobody hello");
    }
    #[test]
    fn spoken_groups() {
        let u = parse_utterance("All shop, stop and commit", &live());
        assert_eq!((u.to, u.rest.as_str()), (vec![0, 1], "stop and commit"));
        let u = parse_utterance("Everyone, wrap up for the night", &live());
        assert_eq!((u.to, u.rest.as_str()), (vec![0, 1, 2], "wrap up for the night"));
        // "all" on its own is an ordinary word
        assert_eq!(parse_utterance("all good, carry on", &live()).to, Vec::<usize>::new());
    }
    #[test]
    fn launching() {
        let projects = vec!["shop".to_string(), "backoffice".to_string(), "ware house".to_string()];
        let l = |t: &str| parse_launch(t, &projects).map(|l| (l.project.unwrap_or_default(), l.task));
        assert_eq!(l("/launch shop write the unit 5 questions"), Some(("shop".into(), "write the unit 5 questions".into())));
        assert_eq!(l("Tower, launch shop: write the unit 5 questions"), Some(("shop".into(), "write the unit 5 questions".into())));
        assert_eq!(l("start a new agent in back office to fix the login bug"), Some(("backoffice".into(), "fix the login bug".into())));
        assert_eq!(l("Spin up an agent for ware house, check today's fakes"), Some(("ware house".into(), "check today's fakes".into())));
        assert_eq!(l("launch shop"), Some(("shop".into(), String::new())));
        // ordinary replies that happen to begin with the same words
        assert_eq!(l("start with the first one"), None);
        assert_eq!(l("start shop on the second passage"), None);
        assert_eq!(l("launch it to production"), None);
        assert_eq!(l("launch a new agent in payroll to do the thing"), None); // not a project the tower knows
        assert_eq!(l("/deploy shop now"), None);
        // only the idea: a launch, with the project left to be worked out
        assert_eq!(l("new idea: warn the warehouse when a part is about to sell out"), Some((String::new(), "warn the warehouse when a part is about to sell out".into())));
        assert_eq!(l("start a new agent to fix the login bug"), Some((String::new(), "fix the login bug".into())));
        assert_eq!(l("spin up an agent, check today's fakes"), Some((String::new(), "check today's fakes".into())));
        // "new" at the start of an ordinary reply is not one
        assert_eq!(l("new questions look fine"), None);
        assert_eq!(l("launch it Friday"), None);
    }
    #[test]
    fn next_callsign() {
        assert_eq!(Callsign::next_for("/x/shop", &live()).text(), "shop three");
        assert_eq!(Callsign::next_for("C:\\work\\My-App\\", &live()).text(), "my app one");
        assert_eq!(live()[0].spoken(), "Shop one");
    }

    #[test]
    fn a_model_and_an_effort_before_the_task() {
        let models = ["fable", "opus", "sonnet"].map(String::from);
        let efforts = ["low", "medium", "high", "xhigh", "max"].map(String::from);
        let w = |t: &str| parse_with(t, &models, &efforts).map(|w| (w.model, w.effort.unwrap_or_default(), w.rest));
        let got = |m: &str, e: &str, r: &str| Some((m.to_string(), e.to_string(), r.to_string()));
        assert_eq!(w("with Fable, I need you to investigate the refund failures"), got("fable", "", "I need you to investigate the refund failures"));
        assert_eq!(w("Hey, with Fable high: look into checkout"), got("fable", "high", "look into checkout"));
        assert_eq!(w("using opus on extra high, compare the two carriers"), got("opus", "xhigh", "compare the two carriers"));
        assert_eq!(w("with sonnet at maximum effort review the importer"), got("sonnet", "max", "review the importer"));
        assert_eq!(w("with opus medium thinking, find the slow query"), got("opus", "medium", "find the slow query"));
        // a model the launcher does not offer, or no model at all
        assert_eq!(w("with gemini, look into checkout"), None);
        assert_eq!(w("with the new carrier it is cheaper"), None);
        assert_eq!(w("investigate checkout with opus"), None);
        // "high" after the model is the effort only when it is one; here it is part of the task
        assert_eq!(w("with opus highlight the slow parts"), got("opus", "", "highlight the slow parts"));

        let asks = |t: &str| parse_with(t, &models, &efforts).is_some_and(|w| w.asks_for_an_agent());
        assert!(asks("with fable, look into the refund failures"));
        assert!(asks("with opus high investigate x y and z"));
        // remarks to whoever he is talking to
        assert!(!asks("with opus it is too slow"));
        assert!(!asks("with sonnet the summaries were fine"));
        assert!(!asks("with fable, yes"));
    }

    #[test]
    fn a_badge_is_the_team_and_the_number_or_name() {
        assert_eq!(badge("shop", "shop three"), "S3");
        assert_eq!(badge("catalog", "catalog part page"), "CP");
        assert_eq!(badge("backoffice", "backoffice f1"), "BF");
        assert_eq!(badge("shop", "shop twelve"), "S12");
        assert_eq!(badge("orders", "orders"), "O");
        // a callsign from before its team was renamed still gets something
        assert_eq!(badge("shop", "catalog import"), "SC");
    }
}
