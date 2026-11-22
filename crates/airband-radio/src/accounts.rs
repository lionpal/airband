//! The coding-tool accounts on each computer, as the host shows them.

/// One allowance of an account: `used` is from 0 to 1, `resets_at` milliseconds since 1970.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AccountLimit {
    pub name: String,
    pub used: f64,
    pub resets_at: Option<u64>,
}

/// A login the agents on one computer can run under. `id` and `harness` are what to pass back to switch to it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AccountEntry {
    pub id: String,
    pub harness: String,
    pub label: String,
    /// The one in use now.
    pub live: bool,
    pub limits: Vec<AccountLimit>,
    /// Why it cannot be used or checked. Show it in place of the limits.
    pub problem: Option<String>,
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

impl StationAccounts {
    pub(crate) fn from(computer: &crate::DeskComputer, a: airband_core::radio::Accounts) -> Self {
        StationAccounts {
            station: computer.id.clone(),
            name: computer.name.clone(),
            accounts: a
                .accounts
                .into_iter()
                .map(|x| AccountEntry { id: x.id, harness: x.harness, label: x.label, live: x.live, limits: x.limits.into_iter().map(|l| AccountLimit { name: l.name, used: l.used, resets_at: l.resets_at }).collect(), problem: x.problem })
                .collect(),
            note: a.note,
            checked_at: a.checked_at,
        }
    }
}
