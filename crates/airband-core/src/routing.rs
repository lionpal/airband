//! Callsigns, and working out what an utterance means and who it is for.

const WORDS: [&str; 13] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve"];

/// A session's name on the radio: its folder's name plus a number word ("shop two"), or plus the name he gave
/// the session ("warehouse removals"). `number` holds whichever follows the project's name.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Callsign {
    pub project: String,
    pub number: String,
}

/// The team a folder or a name he gave stands for: its last part, as a callsign writes it. None when there is
/// nothing in it a team could be called.
pub fn team_name(text: &str) -> Option<String> {
    let base = text.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("");
    Some(norm(base)).filter(|name| !name.is_empty())
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
    /// Is this one of the names the tower hands out by number ("shop two"), and not one he gave?
    pub fn numbered(&self) -> bool {
        !own_name(&self.number)
    }
    /// This callsign in another team. A name he gave it goes with it ("warehouse removals" → "shop removals"); a
    /// number was only its place in the team it leaves, so it takes the next free one in the new team. Nothing
    /// when the team's name says nothing.
    pub fn moved_to(&self, team: &str, live: &[Callsign]) -> Option<Callsign> {
        let project = norm(team);
        if project.is_empty() {
            return None;
        }
        let named = Callsign { project: project.clone(), number: self.number.clone() };
        Some(if own_name(&self.number) && !live.contains(&named) { named } else { Callsign::next_for(&project, live) })
    }
    /// The next free callsign for a session in this folder: the folder's name plus the lowest unused number.
    /// A folder with no name of its own (the top of a disk) is in the team called "session".
    pub fn next_for(cwd: &str, live: &[Callsign]) -> Callsign {
        let project = team_name(cwd).unwrap_or_else(|| "session".into());
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

/// What a radio's own buttons send for each control: a word behind a slash. Only that form is one. What he
/// says or types in his own words is never read as an order to the tower, however much it sounds like one:
/// "go ahead", "skip" and "never mind" are answers he gives agents all day.
const COMMANDS: [(Control, &str); 7] =
    [(Control::Poke, "poke"), (Control::More, "more"), (Control::Repeat, "repeat"), (Control::Status, "status"), (Control::Skip, "skip"), (Control::Hold, "hold"), (Control::Resume, "resume")];

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
/// The tag for every agent: `@all`.
const EVERYONE: &str = "all";

/// Who a typed tag means: `@all`, a whole group (`@shop`), or one agent (`@shop2`, `@shop-two`).
fn tagged(tag: &str, live: &[Callsign]) -> Option<Vec<usize>> {
    let k = key(tag);
    if k == EVERYONE {
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

/// What he typed that needs no reading: who the tags at the front name, and a command if the rest is one.
///
/// Tags: any number of leading ones, `@shop2 @backoffice leave it`, `@shop stop` (the whole group), `@all`.
/// A command: `/more`, `/status` and the rest of [`COMMANDS`].
///
/// Nothing else is read here. Who his own words are for ("shop two, leave it", "everyone, stop") is for a
/// model to say, with the roster in front of it; his words reach the agent as he said them.
pub fn parse_utterance(text: &str, live: &[Callsign]) -> Parsed {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut to: Vec<usize> = Vec::new();
    let mut taken = 0;
    while let Some(hit) = words.get(taken).filter(|w| w.starts_with('@')).and_then(|w| tagged(w, live)) {
        hit.into_iter().for_each(|i| if !to.contains(&i) { to.push(i) });
        taken += 1;
    }
    let rest = words[taken..].join(" ");
    let control = rest.strip_prefix('/').map(|c| c.trim().to_lowercase()).and_then(|c| COMMANDS.iter().find(|(_, name)| *name == c).map(|(c, _)| *c));
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

/// The model and effort of a typed launch, written before the task: `with fable`, `with opus high: …`.
/// `models` and `efforts` are what the launcher in use offers, by the names it lists them under; anything
/// else is part of the task.
pub fn parse_with(text: &str, models: &[String], efforts: &[String]) -> Option<With> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if key(words.first()?) != "with" {
        return None;
    }
    let model = models.iter().find(|m| key(m) == key(words.get(1).copied().unwrap_or_default()))?.clone();
    let effort = words.get(2).and_then(|w| efforts.iter().find(|e| key(e) == key(w)).cloned());
    let at = 2 + usize::from(effort.is_some());
    let rest = words[at..].join(" ").trim_start_matches([' ', ',', '.', ':', ';', '-']).trim().to_string();
    Some(With { model, effort, rest })
}

/// The word a tower numbers an agent with: "one", "two", and figures past twelve.
pub fn number_name(n: usize) -> String {
    WORDS.get(n).map(|w| w.to_string()).unwrap_or_else(|| n.to_string())
}

/// A callsign as a screen shows it: a number said as a word is written as a figure ("shop three" is "shop 3").
/// Aloud it is still the word; either form is understood when he says or types it.
pub fn shown(callsign: &str) -> String {
    match callsign.rsplit_once(' ') {
        Some((team, last)) => format!("{team} {}", shown_number(last)),
        None => callsign.to_string(),
    }
}

/// The part of a callsign after its team, as a screen shows it: "three" is "3", "removals" is "removals".
pub fn shown_number(number: &str) -> String {
    match WORDS.iter().position(|w| *w == number) {
        Some(n) if n > 0 => n.to_string(),
        _ => number.to_string(),
    }
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
    /// The coding tool to start it with, when the command names one (`tool:codex`, right after the project).
    pub tool: Option<String>,
}

/// The typed command that starts a new agent: `/launch shop write the unit 5 questions` (also `/new` and
/// `/start`). The project is one the tower knows, by its name ("back office" is backoffice); with none named,
/// everything after the command is the task and the tower works out where it belongs.
///
/// Only the command is read here. "Start a new agent in shop to …" in his own words is for a model to read.
pub fn parse_launch(text: &str, projects: &[String]) -> Option<Launch> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if !matches!(words.first()?.strip_prefix('/')?.to_lowercase().as_str(), "launch" | "new" | "start") {
        return None;
    }
    // the longest run of the next words that spells a known project
    let mut found: Option<(usize, &String)> = None;
    for n in 1..=3.min(words.len() - 1) {
        let spelled: String = words[1..1 + n].iter().map(|w| key(w)).collect();
        if let Some(p) = projects.iter().find(|p| p.replace(' ', "") == spelled) {
            found = Some((n, p));
        }
    }
    let (n, project) = found.map(|(n, p)| (n, Some(p.clone()))).unwrap_or((0, None));
    // then, in the command's own words and nobody's speech: which tool
    let tool = words.get(1 + n).and_then(|w| w.strip_prefix("tool:")).filter(|t| !t.is_empty()).map(str::to_string);
    let task = words[1 + n + usize::from(tool.is_some())..].join(" ");
    Some(Launch { project, task: task.trim_start_matches([' ', ',', '.', ':', ';', '-']).trim().to_string(), tool })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_team_is_named_by_the_last_part_of_a_folder_and_a_callsign_is_shown_with_a_figure() {
        assert_eq!(team_name("/work/Shop Front/").as_deref(), Callsign::next_for("/work/Shop Front/", &[]).project.as_str().into());
        assert_eq!(team_name("garden").as_deref(), Some("garden"));
        // nothing a team could be called
        assert_eq!((team_name("/"), team_name("  "), team_name("!!")), (None, None, None));
        assert_eq!((shown_number("three"), shown_number("removals"), shown("shop three")), ("3".to_string(), "removals".to_string(), "shop 3".to_string()));
    }

    #[test]
    fn a_session_takes_the_name_he_gave_it() {
        let c = |p: &str, n: &str| Callsign { project: p.into(), number: n.into() };
        assert_eq!(Callsign::titled("/x/shop", "shop 2"), Some(c("shop", "two")));
        assert_eq!(Callsign::titled("/x/warehouse", "warehouse removals"), Some(c("warehouse", "removals")));
        assert_eq!(Callsign::titled("/x/backoffice", "Back Office: login fix"), Some(c("backoffice", "login fix")));
        assert_eq!(Callsign::titled("/x/shop", "curriculum"), Some(c("shop", "curriculum")));
        assert_eq!(Callsign::titled("/x/shop", "shop"), None);
        let live = [c("warehouse", "removals"), c("warehouse", "one"), c("backoffice", "login fix")];
        // only a tag addresses: his own words are not read here
        assert!(parse_utterance("warehouse removals, stop", &live).to.is_empty());
        assert!(parse_utterance("@one stop", &live).to.is_empty());
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
    fn his_own_words_address_nobody_here() {
        // who these are for is a model's to say, and they reach the agent as he said them
        for said in ["Shop two, rewrite both questions.", "shop 2 leave them", "back office, ship it", "Everyone, wrap up for the night", "All shop, stop and commit", "Tower, status", "shop for the missing tests", "yes go ahead and fix it"] {
            assert_eq!(p(said), (None, None, said.to_string()), "{said}");
        }
    }
    #[test]
    fn controls() {
        assert_eq!(p("/more"), (None, Some(Control::More), String::new()));
        assert_eq!(p("/status"), (None, Some(Control::Status), String::new()));
        assert_eq!(p("@shop2 /repeat"), (Some(1), Some(Control::Repeat), String::new()));
        assert_eq!(p("/hold"), (None, Some(Control::Hold), String::new()));
        assert_eq!(p("/Poke"), (None, Some(Control::Poke), String::new()));
    }
    #[test]
    fn his_own_words_are_never_an_order_to_the_tower() {
        for said in ["go ahead", "Skip.", "never mind", "next", "more", "say more", "stand by", "status", "poke", "who's waiting", "nothing", "hold"] {
            assert_eq!(p(said).1, None, "{said}");
            assert!(!p(said).2.is_empty(), "{said} must be passed on");
        }
        assert_eq!(p("skip the second question and repeat the check").1, None);
    }
    #[test]
    fn a_tag_alone() {
        assert_eq!(p("@shop1"), (Some(0), None, String::new()));
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
    fn launching() {
        let projects = vec!["shop".to_string(), "backoffice".to_string(), "ware house".to_string()];
        let l = |t: &str| parse_launch(t, &projects).map(|l| (l.project.unwrap_or_default(), l.task));
        // the command can name the tool, right after the project; said anywhere else it is his task's words
        let with = parse_launch("/launch shop tool:codex fix the cart", &projects).unwrap();
        assert_eq!((with.project.as_deref(), with.tool.as_deref(), with.task.as_str()), (Some("shop"), Some("codex"), "fix the cart"));
        assert_eq!(parse_launch("/launch shop fix tool:codex", &projects).unwrap().tool, None);
        assert_eq!(l("/launch shop write the unit 5 questions"), Some(("shop".into(), "write the unit 5 questions".into())));
        assert_eq!(l("/new back office: fix the login bug"), Some(("backoffice".into(), "fix the login bug".into())));
        assert_eq!(l("/launch ware house check today's fakes"), Some(("ware house".into(), "check today's fakes".into())));
        assert_eq!(l("/launch shop"), Some(("shop".into(), String::new())));
        // no project named: the task, with the project left to be worked out
        assert_eq!(l("/launch warn the warehouse when a part is about to sell out"), Some((String::new(), "warn the warehouse when a part is about to sell out".into())));
        // only the command is one: the same thing in his own words is for a model to read
        for said in ["launch shop: write the unit 5 questions", "start a new agent in back office to fix the login bug", "new idea: warn the warehouse", "start with the first one", "/deploy shop now"] {
            assert_eq!(l(said), None, "{said}");
        }
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
        assert_eq!(w("with fable high: look into checkout"), got("fable", "high", "look into checkout"));
        assert_eq!(w("with opus xhigh compare the two carriers"), got("opus", "xhigh", "compare the two carriers"));
        // a model the launcher does not offer, or no model at all
        assert_eq!(w("with gemini, look into checkout"), None);
        assert_eq!(w("with the new carrier it is cheaper"), None);
        assert_eq!(w("investigate checkout with opus"), None);
        // "high" after the model is the effort only when it is one; here it is part of the task
        assert_eq!(w("with opus highlight the slow parts"), got("opus", "", "highlight the slow parts"));
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
