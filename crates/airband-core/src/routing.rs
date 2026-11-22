//! Callsigns, and working out what an utterance means and who it is for.

const WORDS: [&str; 13] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve"];

/// A session's name on the radio: its folder's name plus a number word ("shop two"), or plus the name he gave
/// the session ("warehouse removals"). `number` holds whichever follows the project's name.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Callsign {
    pub project: String,
    pub number: String,
    /// What follows the project's name is a name he gave the session, not a number the tower handed out. Set
    /// where the callsign is made; never worked out from how it reads.
    #[serde(default)]
    pub named: bool,
}

/// The team a folder or a name he gave stands for: its last part, as a callsign writes it. None when there is
/// nothing in it a team could be called.
pub fn team_name(text: &str) -> Option<String> {
    let base = text.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("");
    Some(norm(base)).filter(|name| !name.is_empty())
}

/// Two callsigns are the same when they read the same: no two agents are called alike, however each came by
/// its name.
impl PartialEq for Callsign {
    fn eq(&self, other: &Self) -> bool {
        self.project == other.project && self.number == other.number
    }
}
impl Eq for Callsign {}

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
        (!rest.is_empty()).then(|| Callsign { project, number: rest.join(" "), named: true })
    }
    /// Is this one of the names the tower hands out by number ("shop two"), and not one he gave?
    pub fn numbered(&self) -> bool {
        !self.named
    }
    /// This callsign in another team. A name he gave it goes with it ("warehouse removals" → "shop removals"); a
    /// number was only its place in the team it leaves, so it takes the next free one in the new team. Nothing
    /// when the team's name says nothing.
    pub fn moved_to(&self, team: &str, live: &[Callsign]) -> Option<Callsign> {
        let project = norm(team);
        if project.is_empty() {
            return None;
        }
        let named = Callsign { project: project.clone(), number: self.number.clone(), named: true };
        Some(if self.named && !live.contains(&named) { named } else { Callsign::next_for(&project, live) })
    }
    /// The next free callsign for a session in this folder: the folder's name plus the lowest unused number.
    /// A folder with no name of its own (the top of a disk) is in the team called "session".
    pub fn next_for(cwd: &str, live: &[Callsign]) -> Callsign {
        let project = team_name(cwd).unwrap_or_else(|| "session".into());
        let mut n = 1;
        loop {
            let number = WORDS.get(n).map(|w| w.to_string()).unwrap_or_else(|| n.to_string());
            if !live.iter().any(|c| c.project == project && c.number == number) {
                return Callsign { project, number, named: false };
            }
            n += 1;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, PartialEq, Eq)]
pub struct Parsed {
    /// Indexes into the callsigns given: everyone this was addressed to. Empty when nobody was named.
    pub to: Vec<usize>,
    /// The words meant for the agents, in his own wording.
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
            k == project || k == format!("{project}{number}") || k == format!("{project}{}", digits(&c.number)) || (c.named && k == number)
        })
        .map(|(i, _)| i)
        .collect();
    (!hits.is_empty()).then_some(hits)
}

/// What he typed that needs no reading: who the tags at the front name.
///
/// Tags: any number of leading ones, `@shop2 @backoffice leave it`, `@shop stop` (the whole group), `@all`.
/// A control is never text: a radio's button sends it as what it is ([`crate::radio::Act`]).
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
    Parsed { to, rest: words[taken..].join(" ") }
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
        let c = |p: &str, n: &str| Callsign { project: p.into(), number: n.into(), named: !WORDS.contains(&n) };
        // a name he gave is one whatever it reads like: "shop 2" is his name for it, not the tower's number
        let t = |p: &str, n: &str| Callsign { project: p.into(), number: n.into(), named: true };
        assert_eq!(Callsign::titled("/x/shop", "shop 2"), Some(t("shop", "two")));
        assert_eq!(Callsign::titled("/x/warehouse", "warehouse removals"), Some(t("warehouse", "removals")));
        assert_eq!(Callsign::titled("/x/backoffice", "Back Office: login fix"), Some(t("backoffice", "login fix")));
        assert_eq!(Callsign::titled("/x/shop", "curriculum"), Some(t("shop", "curriculum")));
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
        [("shop", "one"), ("shop", "two"), ("backoffice", "one")].iter().map(|(p, n)| Callsign { project: p.to_string(), number: n.to_string(), named: false }).collect()
    }
    fn p(t: &str) -> (Option<usize>, String) {
        let u = parse_utterance(t, &live());
        (u.session(), u.rest)
    }

    #[test]
    fn his_own_words_address_nobody_here_and_are_never_an_order_to_the_tower() {
        // who these are for is a model's to say, and they reach the agent as he said them. A word behind a
        // slash is words too: a control comes from a button, as what it is.
        for said in ["Shop two, rewrite both questions.", "shop 2 leave them", "back office, ship it", "Everyone, wrap up for the night", "Tower, status", "yes go ahead and fix it", "skip", "/more", "/launch shop fix it", "/poke"] {
            assert_eq!(p(said), (None, said.to_string()), "{said}");
        }
    }
    #[test]
    fn a_tag_alone() {
        assert_eq!(p("@shop1"), (Some(0), String::new()));
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
    fn next_callsign() {
        assert_eq!(Callsign::next_for("/x/shop", &live()).text(), "shop three");
        assert_eq!(Callsign::next_for("C:\\work\\My-App\\", &live()).text(), "my app one");
        assert_eq!(live()[0].spoken(), "Shop one");
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
