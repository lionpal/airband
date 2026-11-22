//! A stand-in tower: what the desk does with a computer's typed answers, tested without a computer or a
//! network. Each endpoint is scripted with what the real tower would answer, as core's own types; anything
//! not scripted is a computer that does not answer.
use super::*;
use crate::link::{LinkError, Method, Transport, Way};
use airband_core::radio::wire::{Endpoint, Refusal};
use serde_json::Value;

/// What one endpoint is scripted to answer: a status and a body.
type Scripted = (u16, Value);

#[derive(Default)]
pub(crate) struct FakeTower {
    /// What each endpoint answers, by its method and path pattern.
    answers: Mutex<HashMap<(String, &'static str), Scripted>>,
    /// Every request it was asked, in order: method and whole path.
    pub(crate) asked: Mutex<Vec<(String, String)>>,
    /// While set it answers nothing at all, as a computer that is off.
    pub(crate) away: std::sync::atomic::AtomicBool,
}

impl FakeTower {
    /// `E` answers `answer`, as the tower does when it has one.
    pub(crate) fn answers<E: Endpoint>(&self, answer: &E::Answer) -> &Self {
        self.script::<E>(200, serde_json::to_value(answer).expect("an answer writes as JSON"))
    }
    /// `E` is refused, with this status and refusal.
    pub(crate) fn refuses<E: Endpoint>(&self, status: u16, refusal: &Refusal) -> &Self {
        self.script::<E>(status, serde_json::to_value(refusal).expect("a refusal writes as JSON"))
    }
    fn script<E: Endpoint>(&self, status: u16, body: Value) -> &Self {
        self.answers.lock().unwrap().insert((E::METHOD.to_string(), E::PATH), (status, body));
        self
    }
    /// How many times it was asked anything on this endpoint.
    pub(crate) fn asked_on<E: Endpoint>(&self) -> usize {
        self.asked.lock().unwrap().iter().filter(|(m, p)| *m == E::METHOD.to_string() && fits(E::PATH, p)).count()
    }
}

/// Whether a whole path (with its query) is one of this pattern's: a `{…}` part stands for any one part.
fn fits(pattern: &str, path: &str) -> bool {
    let path = path.split('?').next().unwrap_or_default();
    let (a, b): (Vec<&str>, Vec<&str>) = (pattern.split('/').collect(), path.split('/').collect());
    a.len() == b.len() && a.iter().zip(&b).all(|(p, q)| (p.starts_with('{') && p.ends_with('}')) || p == q)
}

impl Transport for FakeTower {
    fn exchange(&self, method: Method, path: &str, _: Option<Value>) -> Result<(u16, Value), LinkError> {
        if self.away.load(std::sync::atomic::Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(20));
            return Err(LinkError::Unreachable(Way::Paired));
        }
        self.asked.lock().unwrap().push((method.to_string(), path.to_string()));
        let answers = self.answers.lock().unwrap();
        let found = answers.iter().find(|((m, p), _)| *m == method.to_string() && fits(p, path)).map(|(_, a)| a.clone());
        // a long wait the stand-in has nothing for: a moment's pause, as the real one holds it open
        if found.is_none() {
            drop(answers);
            thread::sleep(Duration::from_millis(20));
        }
        found.ok_or(LinkError::Unreachable(Way::Paired))
    }
}

impl Desk {
    /// A desk whose computers are reached through the stand-ins `towers` gives for their links.
    pub(crate) fn through(data_dir: &str, host: Arc<dyn DeskHost>, towers: impl Fn(&str) -> Arc<FakeTower> + Send + Sync + 'static) -> Desk {
        let chooser = crate::choices::Chooser::open(data_dir, host.device());
        let dial: Dial = Arc::new(move |link: &str| towers(link) as Arc<dyn Transport>);
        Desk { shared: Arc::new(Shared { state: Mutex::new(State::default()), floor: Floor::default(), host, chooser, senses: Arc::default(), key_test: crate::keys::pretend, dial: Some(dial) }), data_dir: data_dir.into() }
    }
}
