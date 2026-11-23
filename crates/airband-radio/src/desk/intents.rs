//! What every computer is to be told, kept on this phone until each has answered: where this device can be
//! woken, and which coding tools he uses. What writes his summaries is kept the same way with his other
//! choices (`choices.json`). Each computer's answer is kept by its id, so one that says no changes nothing
//! for the others, and one not reached is told when it is.
use super::fanout::Reach;
use super::*;
use airband_core::radio::PushTarget;
use airband_io::kept::Keeper;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where one computer stands with what is wanted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum Delivery {
    /// Not answered yet: it is sent when the computer is reached.
    Pending,
    Done,
    /// It said no, or could not take it, with why in words for him. Not asked again until what is wanted
    /// changes.
    Refused { reason: String },
    /// It could not be sent it as things are (it needs a key that neither it nor this phone has), with why in
    /// words for him. Shown as a no, and tried again whenever it is reached, since that may have changed.
    Held { reason: String },
}

/// Something every computer is to have, with how each stands with it, by the computer's id. A computer not
/// listed has not been asked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Intent<T> {
    /// Which one this is: one more each time what is wanted changes, so an answer to an older one changes
    /// nothing.
    pub(crate) rev: u64,
    pub(crate) wanted: T,
    #[serde(default)]
    pub(crate) per: BTreeMap<StationId, Delivery>,
}

impl<T> Intent<T> {
    /// The next one after `before`, wanting `wanted`, with no computer asked yet.
    pub(crate) fn after(before: Option<&Intent<T>>, wanted: T) -> Intent<T> {
        Intent { rev: before.map_or(1, |b| b.rev + 1), wanted, per: BTreeMap::new() }
    }
    /// That computer has not answered it yet: it is still to be sent.
    pub(crate) fn owed(&self, computer: &StationId) -> bool {
        matches!(self.per.get(computer), None | Some(Delivery::Pending | Delivery::Held { .. }))
    }
    /// Keep what that computer did with it.
    pub(crate) fn record<R>(&mut self, computer: &StationId, reach: &Reach<R>) {
        let now = match reach {
            Reach::Done(_) => Delivery::Done,
            Reach::Unreachable => Delivery::Pending,
            Reach::Refused(why) => Delivery::Refused { reason: why.clone() },
            Reach::Skipped(why) => Delivery::Held { reason: why.clone() },
        };
        self.per.insert(computer.clone(), now);
    }
    /// How it stands as a whole among the computers paired now (`paired`, by id): taken by one at least, still
    /// waiting on one, or refused by every one it was put to, with each one's why. What a computer since
    /// unpaired did counts for nothing.
    pub(crate) fn standing(&self, paired: &[StationId]) -> Standing {
        let per: Vec<&Delivery> = self.per.iter().filter(|(c, _)| paired.contains(c)).map(|(_, d)| d).collect();
        if per.iter().any(|d| **d == Delivery::Done) {
            return Standing::Taken;
        }
        let refused: Vec<String> = per.iter().filter_map(|d| match d { Delivery::Refused { reason } | Delivery::Held { reason } => Some(reason.clone()), _ => None }).collect();
        if per.is_empty() || refused.len() < per.len() { Standing::Waiting } else { Standing::Refused(refused) }
    }
}

/// How something wanted stands as a whole ([`Intent::standing`]).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Standing {
    Taken,
    Waiting,
    Refused(Vec<String>),
}

/// What every computer is to be told about this device.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Intents {
    /// Where this device can be woken with a notification.
    #[serde(default)]
    pub(crate) push: Option<Intent<PushTarget>>,
    /// The coding tools he uses, the one he uses most first.
    #[serde(default)]
    pub(crate) tools: Option<Intent<Vec<String>>>,
}

impl Intents {
    /// Every computer is to have `wanted` for this kind: a new one when it differs from what is wanted now,
    /// else nothing. True when it changed.
    fn want<T: PartialEq>(slot: &mut Option<Intent<T>>, wanted: T) -> bool {
        if slot.as_ref().is_some_and(|i| i.wanted == wanted) {
            return false;
        }
        *slot = Some(Intent::after(slot.as_ref(), wanted));
        true
    }
    pub(crate) fn want_push(&mut self, target: PushTarget) -> bool {
        Self::want(&mut self.push, target)
    }
    pub(crate) fn want_tools(&mut self, tools: Vec<String>) -> bool {
        Self::want(&mut self.tools, tools)
    }
}

impl crate::kept::Rejoin for Intents {
    /// What was wanted since the file could not be read is what is wanted now, after anything the file
    /// held, so an answer to the file's older one changes nothing; the answers it has had are kept.
    fn rejoin(&mut self, since: Intents) {
        fn slot<T: PartialEq>(found: &mut Option<Intent<T>>, since: Option<Intent<T>>) {
            let Some(mut since) = since else { return };
            match found {
                Some(kept) if kept.wanted == since.wanted => kept.per.extend(since.per),
                _ => {
                    since.rev = since.rev.max(found.as_ref().map_or(0, |f| f.rev)) + 1;
                    *found = Some(since);
                }
            }
        }
        slot(&mut self.push, since.push);
        slot(&mut self.tools, since.tools);
    }
}

/// The intents, as this device keeps them (`intents.json`). Kept nowhere in a test with no folder.
#[derive(Default)]
pub(crate) struct KeptIntents {
    pub(crate) now: Intents,
    keeper: Option<Keeper>,
    /// Words for him when the file could not be read as the app opened: told once.
    pub(crate) opened: Option<String>,
}

impl KeptIntents {
    pub(crate) fn open(dir: &str) -> KeptIntents {
        let (now, keeper, opened) = crate::kept::open(crate::kept::file(dir, "intents.json"), "What your computers were told");
        KeptIntents { now, keeper: Some(keeper), opened }
    }
    /// Change them and keep them. The words for him when they could not be kept, the first time in a row.
    pub(crate) fn change<R>(&mut self, f: impl FnOnce(&mut Intents) -> R) -> (R, Option<String>) {
        if let Some(k) = &self.keeper {
            crate::kept::again(k, &mut self.now);
        }
        let out = f(&mut self.now);
        let lost = self.keeper.as_ref().and_then(|k| crate::kept::told(k.save(&self.now), "Your computers may be told again what they already know."));
        (out, lost)
    }
}

/// How each computer stands with one thing wanted, for the view: by name, in his order of computers. Only
/// the ones that do not have it yet.
pub(super) fn lines<T>(intent: &Intent<T>, state: &State) -> Vec<DeskReachLine> {
    state
        .computers
        .iter()
        .filter_map(|c| {
            let id = c.id();
            let state_now = match intent.per.get(&id) {
                Some(Delivery::Done) => return None,
                Some(Delivery::Refused { reason } | Delivery::Held { reason }) => DeskDelivery::Refused { reason: reason.clone() },
                Some(Delivery::Pending) | None => DeskDelivery::Waiting,
            };
            Some(DeskReachLine { computer: id.to_string(), name: state.computer_name(&id), state: state_now })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kept::Rejoin;

    #[test]
    fn what_was_wanted_while_the_file_could_not_be_read_is_what_is_wanted() {
        // (audit 8, M4)
        let target = |t: &str| PushTarget { provider: airband_core::relay::PushService::Apns, token: t.into() };
        let mut found = Intents { push: Some(Intent { rev: 5, wanted: target("old"), per: BTreeMap::new() }), tools: Some(Intent { rev: 2, wanted: vec!["codex".into()], per: BTreeMap::from([(StationId::new("a"), Delivery::Done)]) }) };
        let since = Intents { push: Some(Intent::after(None, target("new"))), tools: Some(Intent { rev: 1, wanted: vec!["codex".into()], per: BTreeMap::from([(StationId::new("b"), Delivery::Done)]) }) };
        found.rejoin(since);
        let push = found.push.unwrap();
        assert_eq!((push.wanted.token.as_str(), push.rev), ("new", 6), "an answer to the file's older one would count for the new one");
        let tools = found.tools.unwrap();
        assert_eq!((tools.rev, tools.per.len()), (2, 2));
    }

    #[test]
    fn only_computers_paired_now_count_and_one_that_needed_a_key_is_asked_again() {
        let (a, b) = (StationId::new("a"), StationId::new("b"));
        let mut i = Intent::after(None, ());
        i.record::<()>(&a, &Reach::Done(()));
        i.record::<()>(&b, &Reach::Refused("No.".into()));
        // (audit 7, P-N3) a computer since unpaired that took it does not make it taken
        assert_eq!(i.standing(&[b.clone()]), Standing::Refused(vec!["No.".into()]));
        assert_eq!(i.standing(&[a.clone(), b.clone()]), Standing::Taken);
        assert_eq!(i.standing(&[]), Standing::Waiting);
        // (audit 7, P-N8) one that could not be sent it for want of a key shows why, and is tried again
        i.record::<()>(&b, &Reach::Skipped("Enter your key.".into()));
        assert!(i.owed(&b));
        assert_eq!(i.standing(&[b.clone()]), Standing::Refused(vec!["Enter your key.".into()]));
        i.record::<()>(&b, &Reach::Refused("No.".into()));
        assert!(!i.owed(&b));
    }
}
