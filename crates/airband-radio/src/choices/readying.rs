//! How far the listening model chosen for this phone has got towards being ready, from facts only: the bytes
//! of its download, then each step its engine says it has done (for a Core ML model, each encoder compiled for
//! this phone). Nothing moves between two facts: a step that has nothing to measure it by is shown as under
//! way, over the part of the ring it will fill, until the engine says it is done.
use super::*;

/// The part of the ring a download fills, for a model that is got ready after it. Its 343 MB take a minute or
/// two on a good connection, and getting it ready after that took 5 to 12 minutes on a phone (`docs/state.md`):
/// the rest is the engine's, each of its steps an equal part.
const DOWNLOAD_PART: f64 = 0.2;

/// Where getting the model ready has got.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum ReadyStep {
    /// Its download waits for another model's to end: one is fetched at a time.
    WaitingTurn,
    /// Its download waits for a network the phone can use.
    WaitingNetwork,
    /// Its files are coming: `got` of `of` bytes.
    Downloading { got: u64, of: u64 },
    /// The engine is getting it ready: `done` of its `of` steps. `of` is 0 until the engine has said how
    /// many there are.
    Preparing { done: u32, of: u32 },
}

/// The listening model he chose is getting ready, and how far it has got: for the ring around the
/// microphone, a line under the composer and on its card, and what he is told when he talks meanwhile.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Readying {
    pub step: ReadyStep,
    /// How much of the ring is done, from 0 to 1.
    pub done: f64,
    /// Where on the ring the step under way ends. Past `done` when that step has nothing to measure it by (a
    /// wait, or the engine at work on one of its steps): that span is shown as under way, never as filling.
    /// Equal to `done` while a download fills the ring byte by byte.
    pub step_ends: f64,
    /// One short line: how far it has got.
    pub line: String,
    /// What he is told when he opens the microphone before it is ready: what he says is kept, and heard
    /// once it is.
    pub talking: String,
}

impl Readying {
    pub(super) fn of(step: ReadyStep, m: &Model) -> Readying {
        let download = if crate::models::needs_preparing(m) { DOWNLOAD_PART } else { 1.0 };
        let (done, step_ends) = match step {
            ReadyStep::WaitingTurn | ReadyStep::WaitingNetwork => (0.0, download),
            ReadyStep::Downloading { got, of } => {
                let d = if of == 0 { 0.0 } else { download * got.min(of) as f64 / of as f64 };
                (d, d)
            }
            // how many steps there are is not said yet: the whole of the engine's part is under way
            ReadyStep::Preparing { of: 0, .. } => (download, 1.0),
            ReadyStep::Preparing { done, of } => {
                let each = (1.0 - download) / of as f64;
                (download + each * done as f64, (download + each * (done + 1) as f64).min(1.0))
            }
        };
        let percent = (done * 100.0).floor() as u32;
        let line = match step {
            ReadyStep::WaitingTurn => "Listening waits for the other download".to_string(),
            ReadyStep::WaitingNetwork => "Listening waits for the network".to_string(),
            _ => format!("Getting listening ready \u{b7} {percent}%"),
        };
        let talking = format!("Listening on this phone is getting ready ({percent}%). Your words will show up when it\u{2019}s done.");
        Readying { step, done, step_ends, line, talking }
    }
}

/// What the ring and its lines show at this step for the listening model this build starts with, whatever this
/// phone has: for a development build's screenshot of each step (`-airband.devReadying`).
#[uniffi::export]
pub fn readying_shown(step: ReadyStep) -> Option<Readying> {
    let m = airband_core::catalog::current().models.iter().find(|m| m.job == Job::Ears && crate::models::needs_preparing(m))?.clone();
    Some(Readying::of(step, &m))
}

impl Chooser {
    /// How far the model being got ready for this job has got, when one is: its download, or its engine's
    /// steps. Nothing for a service, and nothing when no work is being done for the job.
    pub(super) fn readying_for(&self, job: ChoiceJob) -> Option<(Arc<Model>, Readying)> {
        let w = self.working.lock().unwrap_or_else(|e| e.into_inner()).get(&job).cloned()?;
        let m = self.model(&w.id).filter(|m| m.job == job.job())?;
        let step = if w.fetching {
            let p = crate::models::local_model_progress();
            match () {
                _ if p.kind.as_deref() != Some(m.kind.as_str()) => ReadyStep::WaitingTurn,
                _ if p.offline && !p.whole => ReadyStep::WaitingNetwork,
                _ => ReadyStep::Downloading { got: if p.whole { p.of } else { p.got.min(p.of) }, of: p.of },
            }
        } else {
            let (done, of) = crate::models::steps_in(&crate::models::folder(&self.dir, &m)).unwrap_or((0, 0));
            ReadyStep::Preparing { done, of }
        };
        let r = Readying::of(step, &m);
        Some((m, r))
    }

    /// The listening model he chose is getting ready: how far it has got. Nothing when what he chose to listen
    /// with is not a model on this phone, or is one that is not being got ready now (ready, or waiting for
    /// him to download it or try again).
    pub(crate) fn mic_readying(&self) -> Option<Readying> {
        let pick = self.saved().pick(ChoiceJob::Ears).filter(|p| p.place == ChoicePlace::Phone)?;
        self.readying_for(ChoiceJob::Ears).filter(|(m, _)| m.kind == pick.id).map(|(_, r)| r)
    }

    /// A model chosen for this job is on its way to this device: its download is under way or waits its turn.
    pub(super) fn on_its_way(&self, job: ChoiceJob, m: &Model) -> bool {
        self.working.lock().unwrap_or_else(|e| e.into_inner()).get(&job).is_some_and(|w| w.fetching && w.id == m.kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(prepare: bool) -> Model {
        let with = if prepare { [(crate::models::field::PREPARE.to_string(), serde_json::json!("Getting it ready"))].into() } else { Default::default() };
        Model { kind: "tiny".into(), name: "Tiny".into(), job: Job::Ears, engine: "host-only".into(), version: "1".into(), base: "https://airband.dev/models/tiny".into(), files: vec![("a.bin".into(), 3, "0".repeat(64))], voices: Vec::new(), usual_voice: String::new(), credit: String::new(), with }
    }

    #[test]
    fn the_ring_moves_only_on_facts_and_a_step_with_no_measure_is_shown_as_under_way() {
        let m = model(true);
        let at = |step| Readying::of(step, &m);
        // waiting for its turn, or for a network: nothing done, the download's part under way
        assert_eq!((at(ReadyStep::WaitingTurn).done, at(ReadyStep::WaitingTurn).step_ends), (0.0, DOWNLOAD_PART));
        assert_eq!(at(ReadyStep::WaitingNetwork).line, "Listening waits for the network");
        // a download fills its part byte by byte, with nothing under way past it
        let half = at(ReadyStep::Downloading { got: 50, of: 100 });
        assert_eq!((half.done, half.step_ends), (DOWNLOAD_PART / 2.0, DOWNLOAD_PART / 2.0));
        assert_eq!(half.line, "Getting listening ready \u{b7} 10%");
        // the engine has not said how many steps: the whole of its part is under way, and nothing more is done
        let unsaid = at(ReadyStep::Preparing { done: 0, of: 0 });
        assert_eq!((unsaid.done, unsaid.step_ends), (DOWNLOAD_PART, 1.0));
        // three steps: each an equal part, the one under way shown as such, never as filling
        let each = (1.0 - DOWNLOAD_PART) / 3.0;
        let one = at(ReadyStep::Preparing { done: 1, of: 3 });
        assert!((one.done - (DOWNLOAD_PART + each)).abs() < 1e-9 && (one.step_ends - (DOWNLOAD_PART + 2.0 * each)).abs() < 1e-9);
        assert_eq!(one.line, "Getting listening ready \u{b7} 46%");
        assert_eq!(one.talking, "Listening on this phone is getting ready (46%). Your words will show up when it\u{2019}s done.");
        let all = at(ReadyStep::Preparing { done: 3, of: 3 });
        assert_eq!((all.done, all.step_ends), (1.0, 1.0));
        // a model with nothing to get ready after its download: the download is the whole ring
        let plain = Readying::of(ReadyStep::Downloading { got: 1, of: 2 }, &model(false));
        assert_eq!(plain.done, 0.5);
    }
}
