//! The watcher of the settings: a pairing added by `airband pair` starts being served, one removed by `airband
//! unpair` stops, and a changed relay address moves everything, all without restarting the tower.
use super::super::{App, log};
use super::{Ended, Serve};
use crate::config::PairedRadio;
use axum::Router;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::runtime::Handle;

pub(crate) fn start(app: Arc<App>, router: Router, handle: Handle) {
    std::thread::spawn(move || {
        let mut links = Links::default();
        loop {
            links.watch(&app, &router, &handle);
            std::thread::sleep(Duration::from_secs(10));
        }
    });
}

/// A link: its relay and its pairing.
pub(super) type Link = (Option<String>, PairedRadio);

/// The links the watcher keeps: each that is up, with the way to stop it, and those that ended for good, never
/// opened again while that pairing stands.
#[derive(Default)]
pub(super) struct Links {
    pub(super) running: HashMap<Link, (Arc<AtomicBool>, std::thread::JoinHandle<Ended>)>,
    finished: HashSet<Link>,
}

impl Links {
    /// One look at the settings: start what is wanted, stop what is not.
    pub(super) fn watch(&mut self, app: &Arc<App>, router: &Router, handle: &Handle) {
        // the settings now, with their keys: ones read without them never take the place of these
        let cfg = app.cfg();
        // every pairing is served straight, at the tower's own addresses; through the relay too when there is one
        let wanted: Vec<Link> = cfg.pairings.iter().map(|p| (cfg.relay.clone(), p.clone())).collect();
        self.finished.retain(|key| wanted.contains(key));
        self.join_ended(app, &wanted);
        self.running.retain(|key, (stop, _)| {
            let keep = wanted.contains(key);
            if !keep {
                stop.store(true, Ordering::SeqCst);
                app.served.forget(&key.1);
                if let Some(channel) = super::super::push::channel_of(&key.1) {
                    log(format!("{} is no longer served", super::super::radio_tag(&channel)));
                }
            }
            keep
        });
        for key in wanted {
            if self.running.contains_key(&key) || self.finished.contains(&key) {
                continue;
            }
            // one that cannot be served now is said, and looked at again on the next look
            let serve = match Serve::new(app, key.0.clone(), &key.1, router.clone(), handle.clone()) {
                Ok(serve) => serve,
                Err(why) => {
                    log(format!("a pairing cannot be served: {why}"));
                    continue;
                }
            };
            app.served.insert(serve.keys.channel.clone(), serve.clone());
            let stop = serve.stop.clone();
            self.running.insert(key, (stop, std::thread::spawn(move || serve.run())));
        }
    }

    /// A pairing whose serving thread has ended while it is still wanted is started again (with nobody pulling,
    /// the relay would call the computer away while it is on), unless it ended for good: the relay keeps the
    /// channel closed, or the pairing is gone.
    fn join_ended(&mut self, app: &Arc<App>, wanted: &[Link]) {
        let ended: Vec<Link> = self.running.iter().filter(|(key, (_, serving))| wanted.contains(key) && serving.is_finished()).map(|(key, _)| key.clone()).collect();
        for key in ended {
            let Some((_, serving)) = self.running.remove(&key) else { continue };
            match serving.join() {
                // ended for good: its straight route goes with it, so a phone no longer paired, or whose
                // channel the relay keeps closed, is not answered on the local network either
                Ok(Ended::Closed | Ended::Unpaired) => {
                    app.served.forget(&key.1);
                    self.finished.insert(key);
                }
                Ok(Ended::Stopped) | Err(_) => log(format!("the link serving \"{}\" stopped; starting it again", key.1.name)),
            }
        }
    }
}
