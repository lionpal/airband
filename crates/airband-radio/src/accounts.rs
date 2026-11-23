//! The coding-tool accounts on each computer, as the host shows them.

/// One allowance of an account: `used` is from 0 to 1, `resets_at` milliseconds since 1970.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AccountLimit {
    pub name: String,
    pub used: f64,
    pub resets_at: Option<u64>,
}

/// Who an account is, as its tool records it: what a host may show of it. Two computers' accounts are one
/// when their tools record the same person (or the same key); the phone works that out, never by the label.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum AccountWho {
    /// The tool records who is signed in.
    Known,
    /// An API key, shown by its last four characters.
    Key { last4: String },
    /// Nothing the tool records says who it is: never taken for another computer's.
    Unknown,
}

impl From<&airband_core::identity::AccountIdentity> for AccountWho {
    fn from(i: &airband_core::identity::AccountIdentity) -> Self {
        use airband_core::identity::AccountIdentity as I;
        match i {
            I::Id { .. } => AccountWho::Known,
            I::Key { last4, .. } => AccountWho::Key { last4: last4.clone() },
            I::None => AccountWho::Unknown,
        }
    }
}

/// A login the agents on one computer can run under. `id` and `harness` are what to pass back to switch to it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AccountEntry {
    pub id: String,
    pub harness: String,
    /// What to show him. Only to show.
    pub label: String,
    pub who: AccountWho,
    /// The one in use now.
    pub live: bool,
    pub limits: Vec<AccountLimit>,
    /// Why it cannot be used or checked. Show it in place of the limits.
    pub problem: Option<String>,
    /// The limits were read on another of his computers, where the same account is: that computer's name.
    pub limits_from: Option<String>,
}

/// One computer's accounts. `note` is something to show under the list.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StationAccounts {
    /// The computer, by its [`crate::DeskComputer::id`], and what it is called to him.
    pub station: String,
    pub name: String,
    pub accounts: Vec<AccountEntry>,
    pub note: Option<String>,
    pub checked_at: u64,
}

fn limits_of(limits: &[airband_core::radio::Limit]) -> Vec<AccountLimit> {
    limits.iter().map(|l| AccountLimit { name: l.name.clone(), used: l.used, resets_at: l.resets_at }).collect()
}

impl StationAccounts {
    pub(crate) fn from(computer: &crate::DeskComputer, a: airband_core::radio::Accounts) -> Self {
        StationAccounts::across(&[(computer.clone(), a)]).remove(0)
    }

    /// Every computer's accounts, as each told them, with one thing more: an account a computer could not read
    /// the limits of takes them from another computer where the same account is, by who its tool says it is
    /// ([`airband_core::identity::AccountIdentity::same`]) and never by what it is called. One with a problem of
    /// its own keeps it.
    pub(crate) fn across(told: &[(crate::DeskComputer, airband_core::radio::Accounts)]) -> Vec<Self> {
        let elsewhere = |here: usize, x: &airband_core::radio::Account| {
            told.iter().enumerate().filter(|(i, _)| *i != here).find_map(|(_, (computer, a))| {
                a.accounts.iter().find(|y| y.harness == x.harness && !y.limits.is_empty() && y.identity.same(&x.identity)).map(|y| (computer.name.clone(), limits_of(&y.limits)))
            })
        };
        told.iter()
            .enumerate()
            .map(|(here, (computer, a))| StationAccounts {
                station: computer.id.clone(),
                name: computer.name.clone(),
                accounts: a
                    .accounts
                    .iter()
                    .map(|x| {
                        let (limits, limits_from) = match (x.limits.is_empty() && x.problem.is_none()).then(|| elsewhere(here, x)).flatten() {
                            Some((from, limits)) => (limits, Some(from)),
                            None => (limits_of(&x.limits), None),
                        };
                        AccountEntry { id: x.id.clone(), harness: x.harness.clone(), label: x.label.clone(), who: AccountWho::from(&x.identity), live: x.live, limits, problem: x.problem.clone(), limits_from }
                    })
                    .collect(),
                note: a.note.clone(),
                checked_at: a.checked_at,
            })
            .collect()
    }
}

/// One coding tool's accounts, over all his computers: what the accounts screen shows under the tool's name.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ToolAccounts {
    /// The tool's id, and what it is called (from the catalogue; its id when the catalogue does not list it).
    pub harness: String,
    pub name: String,
    pub rows: Vec<AccountRow>,
}
/// One account on one computer, as a row.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AccountRow {
    /// The computer, by id, to pass back to switch; and its name, shown only when he has several.
    pub station: String,
    pub computer: String,
    /// What the row says: the account's name, or "Key …ab12" for a key.
    pub shown: String,
    pub entry: AccountEntry,
}

/// Every computer's accounts, grouped by coding tool in the catalogue's order: the accounts screen.
#[uniffi::export]
pub fn accounts_by_tool(accounts: Vec<StationAccounts>) -> Vec<ToolAccounts> {
    let several = accounts.len() > 1;
    let tools = crate::services::coding_tools();
    let mut out: Vec<ToolAccounts> = Vec::new();
    for station in &accounts {
        for entry in &station.accounts {
            let row = AccountRow {
                station: station.station.clone(),
                computer: if several { station.name.clone() } else { String::new() },
                shown: match &entry.who {
                    AccountWho::Key { last4 } => format!("Key \u{2026}{last4}"),
                    _ => entry.label.clone(),
                },
                entry: entry.clone(),
            };
            match out.iter_mut().find(|t| t.harness == entry.harness) {
                Some(t) => t.rows.push(row),
                None => out.push(ToolAccounts { harness: entry.harness.clone(), name: crate::services::tool_name(&entry.harness).unwrap_or_else(|| entry.harness.clone()), rows: vec![row] }),
            }
        }
    }
    let place = |h: &str| tools.iter().position(|t| t.id == h).unwrap_or(usize::MAX);
    out.sort_by_key(|t| place(&t.harness));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use airband_core::identity::AccountIdentity;
    use airband_core::radio::{Account, Accounts, Limit};

    fn computer(id: &str) -> crate::DeskComputer {
        crate::DeskComputer { id: id.into(), name: id.into() }
    }
    fn account(label: &str, identity: AccountIdentity, read: bool) -> Account {
        let limits = if read { vec![Limit { name: "week".into(), used: 0.5, ..Limit::default() }] } else { Vec::new() };
        Account { id: "a".into(), harness: "h".into(), label: label.into(), identity, live: true, limits, problem: None }
    }
    fn on(accounts: Vec<Account>) -> Accounts {
        Accounts { accounts, ..Accounts::default() }
    }
    fn person(id: &str) -> AccountIdentity {
        AccountIdentity::Id { id: id.into(), org: Some("org".into()) }
    }

    #[test]
    fn an_account_takes_the_limits_read_elsewhere_only_when_its_tool_says_it_is_the_same_one() {
        let told = [
            (computer("mini"), on(vec![account("me@example.com", person("p1"), true), account("key", AccountIdentity::of_key("openai", "sk-fake-key-one"), true)])),
            (computer("laptop"), on(vec![
                // the same person: takes mini's limits, and says where they were read
                account("someone else", person("p1"), false),
                // the same label, someone else: keeps none
                account("me@example.com", person("p2"), false),
                // the same key: takes them
                account("…-one", AccountIdentity::of_key("openai", "sk-fake-key-one"), false),
                // nothing says who it is: never taken for anyone
                account("me@example.com", AccountIdentity::None, false),
            ])),
        ];
        let all = StationAccounts::across(&told);
        let laptop = &all[1].accounts;
        assert_eq!(laptop[0].limits.len(), 1);
        assert_eq!(laptop[0].limits_from.as_deref(), Some("mini"));
        assert!(laptop[1].limits.is_empty() && laptop[1].limits_from.is_none());
        assert_eq!(laptop[2].limits_from.as_deref(), Some("mini"));
        assert_eq!(laptop[2].who, AccountWho::Key { last4: "-one".into() });
        assert!(laptop[3].limits.is_empty() && laptop[3].who == AccountWho::Unknown);
        // mini's own are as it told them
        assert!(all[0].accounts.iter().all(|a| a.limits_from.is_none()));
    }

    #[test]
    fn one_person_in_two_workspaces_is_two_accounts() {
        let other_org = AccountIdentity::Id { id: "p1".into(), org: Some("other".into()) };
        let told = [(computer("mini"), on(vec![account("me", person("p1"), true)])), (computer("laptop"), on(vec![account("me", other_org, false)]))];
        assert!(StationAccounts::across(&told)[1].accounts[0].limits.is_empty());
    }

    #[test]
    fn accounts_are_shown_by_tool_and_a_key_by_its_last_four() {
        let mut key = account("x", AccountIdentity::of_key("openai", "sk-fake-abcd1234"), false);
        key.harness = "some-tool".into();
        let told = [(computer("mini"), on(vec![account("me@example.com", person("p1"), true), key]))];
        let shown = accounts_by_tool(StationAccounts::across(&told));
        let names: Vec<(&str, Vec<&str>)> = shown.iter().map(|t| (t.harness.as_str(), t.rows.iter().map(|r| r.shown.as_str()).collect())).collect();
        assert!(names.iter().any(|(h, rows)| *h == "some-tool" && rows == &vec!["Key \u{2026}1234"]), "{names:?}");
        assert!(shown.iter().all(|t| t.rows.iter().all(|r| r.computer.is_empty())), "one computer is never named");
    }
}
