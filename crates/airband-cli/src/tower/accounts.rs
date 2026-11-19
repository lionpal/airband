//! Accounts: the logins the harnesses on this computer can run under, how much of each is used, and switching.
//! The tower only keeps the answer for a while and passes the asking on; what an account is and how it is
//! switched belongs to the harness's adapter (`adapters::accounts`). A radio is told what each account is called
//! and how full it is, never anything that could sign in as it.
use super::*;
use airband_core::radio::{Accounts, UseAccount};

/// How long an answer is good for, and how soon a radio that asks for a fresh one can have another.
const GOOD_FOR: Duration = Duration::from_secs(300);
const FRESH_AFTER: Duration = Duration::from_secs(20);
/// Saved logins are looked at this often even when nobody asks, so one left unused for weeks never lapses.
const LOOK_EVERY: Duration = Duration::from_secs(3 * 3600);

#[derive(Default)]
pub(super) struct Kept(tokio::sync::Mutex<Option<(Instant, Accounts)>>);

async fn read() -> Accounts {
    tokio::task::spawn_blocking(crate::adapters::accounts).await.unwrap_or_default()
}

/// The accounts, from memory when that is recent enough. Held while it is read, so two radios asking at once
/// make one check.
pub(super) async fn list(app: &Arc<App>, fresh: bool) -> Accounts {
    let mut kept = app.accounts.0.lock().await;
    if let Some((at, value)) = kept.as_ref().filter(|(at, _)| at.elapsed() < if fresh { FRESH_AFTER } else { GOOD_FOR }) {
        let _ = at;
        return value.clone();
    }
    let value = read().await;
    *kept = Some((Instant::now(), value.clone()));
    value
}

/// Switch, then say how things stand now. An error is words for him.
pub(super) async fn switch(app: &Arc<App>, to: UseAccount) -> Result<Accounts, String> {
    let mut kept = app.accounts.0.lock().await;
    let UseAccount { harness, id } = to;
    let (h, i) = (harness.clone(), id.clone());
    // the swap is done when this returns; the thread it hands back only holds the name in place afterwards
    tokio::task::spawn_blocking(move || crate::adapters::use_account(&h, &i).map(|_holding| ())).await.map_err(|_| "The switch did not finish.".to_string())??;
    log(format!("{harness} now runs as {id}"));
    let value = read().await;
    *kept = Some((Instant::now(), value.clone()));
    Ok(value)
}

pub(super) async fn keep_alive(app: Arc<App>) {
    loop {
        tokio::time::sleep(LOOK_EVERY).await;
        list(&app, true).await;
    }
}
