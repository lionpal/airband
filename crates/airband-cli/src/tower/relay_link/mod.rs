//! The tower's side of `airband.relay/1`: for each paired radio, hold a channel open on the relay, pull that
//! radio's sealed requests, run them exactly as if they had arrived on the local port, and seal the answers.
//!
//! The tower only ever calls out, so it works behind any router with no port opened. The relay sees sealed
//! bytes; a request that does not open with a paired radio's key, is not fresh, or was already answered is
//! dropped here.
//!
//! - `said`: what the relay's statuses say, and closing a channel (`airband unpair`)
//! - `sealed`: the route a radio on his own network reaches the tower by, sealed as through the relay
//! - `links`: the watcher that starts and stops a link for each pairing
//! - `pull`: one link's pull loop
//! - `answer`: running a request and sealing its answer
mod answer;
mod links;
mod pull;
mod said;
mod sealed;
pub(crate) use links::start;
pub(crate) use said::{RelaySaid, close_channel};
pub(crate) use sealed::sealed_router;

use super::App;
use crate::config::PairedRadio;
use airband_core::relay::Keys;
use axum::Router;
use airband_core::relay::Pulling;
use pull::Turn;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tokio::runtime::Handle;

/// Why a link's serving thread ended. One the relay keeps closed, or a pairing undone, is not started again
/// while the same pairing is wanted: only a new pairing (a new secret, so a new channel) opens one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ended {
    /// Told to stop: the pairing or the relay address changed.
    Stopped,
    /// The relay keeps this channel closed for good.
    Closed,
    /// The settings no longer hold this pairing.
    Unpaired,
}

/// Every pairing being served, by its channel: a request that comes straight to the tower is answered by the
/// same code, with the same key, as one pulled from the relay. One for each tower, on its [`App`]. Each use is
/// one change to the map, so a holder that panicked left nothing half done in it.
#[derive(Default)]
pub(crate) struct Served(Mutex<HashMap<String, Serve>>);

impl Served {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, Serve>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn get(&self, channel: &str) -> Option<Serve> {
        self.map().get(channel).cloned()
    }
    fn insert(&self, channel: String, serve: Serve) {
        self.map().insert(channel, serve);
    }
    /// A pairing no longer served: its straight route goes with it.
    fn forget(&self, pairing: &PairedRadio) {
        if let Some(channel) = super::push::channel_of(pairing) {
            self.map().remove(&channel);
        }
    }
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.map().is_empty()
    }
}

#[derive(Clone)]
struct Serve {
    app: Arc<App>,
    /// The relay this pairing is also reached through, when there is one.
    relay: Option<String>,
    name: String,
    keys: Keys,
    router: Router,
    handle: Handle,
    stop: Arc<AtomicBool>,
    /// The HTTP client pulls and openings go through: a pull is held up to 25 s.
    pulls: airband_io::http::Client,
    /// The HTTP client answers go back through.
    replies: airband_io::http::Client,
    /// What the next pull says ([`Pulling`]): what the last one got, and the kept message being answered for. An
    /// answer to a kept message that does not reach the relay is not sent again from here: the work on it ends,
    /// the next pull stops naming it, and the relay, which still keeps it, hands it over again.
    pulling: Arc<Mutex<Pulling>>,
    /// Where pulls' outcomes, and word that a kept message was let go, reach the pull loop.
    turns: mpsc::Sender<Turn>,
    inbox: Arc<Mutex<mpsc::Receiver<Turn>>>,
}

impl Serve {
    /// The link for one pairing, pulling under this computer's kept id. Why it cannot be served, when it cannot.
    fn new(app: &Arc<App>, relay: Option<String>, pairing: &PairedRadio, router: Router, handle: Handle) -> Result<Serve, String> {
        let keys = Keys::derive(&pairing.secret).map_err(|e| format!("its key is unusable ({e})"))?;
        let tower = app.store.tower_id().map_err(|e| format!("the id this computer pulls under could not be read ({e})"))?;
        let (turns, inbox) = mpsc::channel();
        Ok(Serve {
            app: app.clone(),
            relay,
            name: pairing.name.clone(),
            keys,
            router,
            handle,
            stop: Arc::new(AtomicBool::new(false)),
            pulls: airband_io::http::Client::new(Duration::from_secs(40)),
            replies: airband_io::http::Client::new(Duration::from_secs(15)),
            pulling: Arc::new(Mutex::new(Pulling::of_tower(tower))),
            turns,
            inbox: Arc::new(Mutex::new(inbox)),
        })
    }

    /// Whether the settings now still hold this pairing, read again from the file when it has changed.
    fn still_paired(&self) -> bool {
        self.app.cfg().pairings.iter().any(|p| super::push::channel_of(p).as_ref() == Some(&self.keys.channel))
    }
}

#[cfg(test)]
mod tests;
