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

/// The most characters a team's name may have: it is part of what its agents are called, and is said aloud.
pub const TEAM_NAME_MOST: usize = 24;

/// A name he chose for a team (a new one, or a rename), as it is kept: [`team_name`], and short. The one check
/// every place he names a team goes through.
pub fn chosen_team_name(text: &str) -> Result<String, &'static str> {
    team_name(text).filter(|n| n.chars().count() <= TEAM_NAME_MOST).ok_or("A team\u{2019}s name is letters and numbers, 24 at most.")
}

/// A name he gave a team, written the way a callsign writes it. Only written so: no part of it is taken for a
/// path ("shop/front" is "shop front"). None when it has no letters or numbers.
pub fn team_name(text: &str) -> Option<String> {
    Some(norm(text)).filter(|name| !name.is_empty())
}

/// The team a folder stands for: the folder's own name, as the system reads the path, the way a callsign
/// writes it. None when there is nothing in it a team could be called (the top of a disk).
pub fn folder_name(path: &str) -> Option<String> {
    team_name(&std::path::Path::new(path.trim()).file_name()?.to_string_lossy())
}

/// The names a team in this folder could take, plainest first: the folder's own name, then its parent's name
/// in front of it, and so on up the path ("shop", "work shop", "a work shop"). Each comes from the path's own
/// parts, as the system reads them. The tower takes the first that no other team of his has.
pub fn team_names(folder: &str) -> Vec<String> {
    let parts: Vec<String> = std::path::Path::new(folder.trim()).components().filter_map(|c| match c {
        std::path::Component::Normal(part) => Some(norm(&part.to_string_lossy())),
        _ => None,
    }).filter(|part| !part.is_empty()).collect();
    (1..=parts.len()).map(|n| parts[parts.len() - n..].join(" ")).collect()
}

/// A team's name with this computer's name in front: for a team whose folder's name a team on another of his
/// computers already has ("laptop shop").
pub fn team_on(computer: &str, team: &str) -> String {
    norm(&format!("{computer} {team}"))
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
    /// The callsign a session's own name asks for, in this team: "shop 2" → shop two, "removals" or
    /// "warehouse removals" → warehouse removals. Nothing when the name says nothing beyond the project.
    pub fn titled(team: &str, folder: &str, title: &str) -> Option<Callsign> {
        let project = team.to_string();
        let title = norm(title);
        // his name for it usually starts with the team's or its folder's ("shop 2", "back office fixes"): that part
        // is not said twice
        let words: Vec<&str> = title.split(' ').filter(|w| !w.is_empty()).collect();
        let leads: Vec<String> = std::iter::once(project.clone()).chain(team_names(folder)).map(|l| l.replace(' ', "")).collect();
        let lead = (1..=words.len()).rev().find(|&n| leads.contains(&words[..n].concat())).unwrap_or(0);
        let rest: Vec<String> = words[lead..].iter().take(4).map(|w| number_word(w)).collect();
        (!rest.is_empty()).then(|| Callsign { project, number: rest.join(" "), named: true })
    }
    /// As a screen shows it: a number as a figure ("shop 3"), a name he gave as he gave it ("warehouse
    /// removals").
    pub fn shown(&self) -> String {
        let rest = self.shown_rest();
        if rest.is_empty() { self.project.clone() } else { format!("{} {rest}", self.project) }
    }
    /// The part after the team, as a screen shows it: "3", or "removals".
    pub fn shown_rest(&self) -> String {
        match figure(&self.number).filter(|_| !self.named) {
            Some(n) => n.to_string(),
            None => self.number.clone(),
        }
    }
    /// Two characters to stand for it where the whole callsign will not fit: the team's initial and its number,
    /// or the initial of the name he gave it. "shop three" is S3, "catalog part page" is CP.
    pub fn badge(&self) -> String {
        let initial = |w: &str| w.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        let second = match figure(&self.number).filter(|_| !self.named) {
            Some(n) => n.to_string(),
            None => initial(self.number.split_whitespace().next().unwrap_or("")),
        };
        format!("{}{}", initial(&self.project), second)
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
        Some(if self.named && !live.contains(&named) { named } else { Callsign::next_in(&project, live) })
    }
    /// The next free callsign in this team: its name plus the lowest unused number.
    pub fn next_in(team: &str, live: &[Callsign]) -> Callsign {
        let project = team.to_string();
        let mut n = 1;
        loop {
            let number = number_name(n);
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
    figure(number).map(|n| n.to_string()).unwrap_or_else(|| number.to_string())
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


/// A number the tower handed out, as a figure: "three" is "3". The tower writes numbers as its own words up
/// to twelve and as figures past that.
fn figure(number: &str) -> Option<u32> {
    WORDS.iter().position(|w| *w == number).map(|n| n as u32).or_else(|| number.parse().ok())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_team_is_named_by_the_last_part_of_a_folder_and_a_callsign_is_shown_with_a_figure() {
        assert_eq!(folder_name("/work/Shop Front/").as_deref(), Some("shop front"));
        assert_eq!(team_name("garden").as_deref(), Some("garden"));
        // a name he typed is never read as a path
        assert_eq!(team_name("shop/front").as_deref(), Some("shop front"));
        assert_eq!(folder_name("/"), None);
        // nothing a team could be called
        assert_eq!((team_name("/"), team_name("  "), team_name("!!")), (None, None, None));
        assert_eq!(team_names("/a/Work/shop/"), ["shop", "work shop", "a work shop"]);
        assert_eq!(team_names("/"), Vec::<String>::new());
        assert_eq!(team_on("Charles's Laptop", "shop"), "charless laptop shop");
        let c = |project: &str, number: &str, named: bool| Callsign { project: project.into(), number: number.into(), named };
        assert_eq!((c("shop", "three", false).shown(), c("warehouse", "removals", true).shown(), c("shop", "13", false).shown()), ("shop 3".to_string(), "warehouse removals".to_string(), "shop 13".to_string()));
    }

    #[test]
    fn a_session_takes_the_name_he_gave_it() {
        let c = |p: &str, n: &str| Callsign { project: p.into(), number: n.into(), named: !WORDS.contains(&n) };
        // a name he gave is one whatever it reads like: "shop 2" is his name for it, not the tower's number
        let t = |p: &str, n: &str| Callsign { project: p.into(), number: n.into(), named: true };
        assert_eq!(Callsign::titled("shop", "/x/shop", "shop 2"), Some(t("shop", "two")));
        assert_eq!(Callsign::titled("warehouse", "/x/warehouse", "warehouse removals"), Some(t("warehouse", "removals")));
        assert_eq!(Callsign::titled("backoffice", "/x/backoffice", "Back Office: login fix"), Some(t("backoffice", "login fix")));
        assert_eq!(Callsign::titled("shop", "/x/shop", "curriculum"), Some(t("shop", "curriculum")));
        assert_eq!(Callsign::titled("shop", "/x/shop", "shop"), None);
        // in a team other than its folder's: neither name is said twice
        assert_eq!(Callsign::titled("returns", "/x/orders", "orders hooks"), Some(t("returns", "hooks")));
        assert_eq!(Callsign::titled("work shop", "/b/work/shop", "shop labels"), Some(t("work shop", "labels")));
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
        assert_eq!(Callsign::next_in("shop", &live()).text(), "shop three");
        assert_eq!(Callsign::next_in("my app", &live()).text(), "my app one");
        assert_eq!(live()[0].spoken(), "Shop one");
    }


    #[test]
    fn a_badge_is_the_team_and_the_number_or_name() {
        let c = |project: &str, number: &str, named: bool| Callsign { project: project.into(), number: number.into(), named };
        assert_eq!(c("shop", "three", false).badge(), "S3");
        assert_eq!(c("catalog", "part page", true).badge(), "CP");
        assert_eq!(c("backoffice", "f1", true).badge(), "BF");
        assert_eq!(c("shop", "twelve", false).badge(), "S12");
        assert_eq!(c("orders", "", false).badge(), "O");
        // a name he gave that reads like a number is still a name
        assert_eq!((c("shop", "one", true).shown(), c("shop", "one", true).badge()), ("shop one".to_string(), "SO".to_string()));
    }
}
