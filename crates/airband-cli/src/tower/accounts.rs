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

/// The accounts as the tools have them now. `Err` with what to tell him when the reading itself ended without an
/// answer (its thread failed): that is not an empty list, and is not kept as one.
async fn read() -> Result<Accounts, Accounts> {
    tokio::task::spawn_blocking(crate::adapters::accounts).await.map_err(|_| Accounts { schema: airband_core::radio::SCHEMA.into(), note: Some("The accounts could not be read just now. Try again in a moment.".into()), ..Accounts::default() })
}

/// The accounts, from memory when that is recent enough. Held while it is read, so two radios asking at once
/// make one check.
pub(super) async fn list(app: &Arc<App>, fresh: bool) -> Accounts {
    let mut kept = app.accounts.0.lock().await;
    if let Some((_, value)) = kept.as_ref().filter(|(at, _)| at.elapsed() < if fresh { FRESH_AFTER } else { GOOD_FOR }) {
        return value.clone();
    }
    match read().await {
        Ok(value) => {
            *kept = Some((Instant::now(), value.clone()));
            value
        }
        Err(told) => told,
    }
}

/// Switch, then say how things stand now. An error is words for him.
pub(super) async fn switch(app: &Arc<App>, to: UseAccount) -> Result<Accounts, String> {
    let mut kept = app.accounts.0.lock().await;
    let UseAccount { harness, id } = to;
    let (h, i) = (harness.clone(), id.clone());
    // the swap is done when this returns; the thread it hands back only holds the name in place afterwards
    tokio::task::spawn_blocking(move || crate::adapters::use_account(&h, &i).map(|_holding| ())).await.map_err(|_| "The switch did not finish.".to_string())??;
    // the account by nothing that names it: its id is his email for a saved login
    log(format!("{harness} now runs as another of his accounts"));
    Ok(match read().await {
        Ok(value) => {
            *kept = Some((Instant::now(), value.clone()));
            value
        }
        // switched, and the list could not be read after: none is kept, so the next ask reads it again
        Err(told) => {
            *kept = None;
            told
        }
    })
}

pub(super) async fn keep_alive(app: Arc<App>) {
    loop {
        tokio::time::sleep(LOOK_EVERY).await;
        list(&app, true).await;
    }
}
