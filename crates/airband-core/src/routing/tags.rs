//! Who a typed `@tag` means, and the controls a radio sends as what they are.
use super::*;

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
    let mut to: Vec<usize> = Vec::new();
    // what is left is a slice of what he wrote, never his words split and joined again: his line breaks,
    // indents and spacing reach the agent as they were
    let mut rest = text.trim_start();
    loop {
        let word = rest.split(char::is_whitespace).next().unwrap_or_default();
        let Some(hit) = Some(word).filter(|w| w.starts_with('@')).and_then(|w| tagged(w, live)) else { break };
        hit.into_iter().for_each(|i| if !to.contains(&i) { to.push(i) });
        rest = rest[word.len()..].trim_start();
    }
    Parsed { to, rest: rest.trim_end().to_string() }
}
