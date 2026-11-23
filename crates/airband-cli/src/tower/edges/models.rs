//! The model that reads for the tower, as the tower asks it. The real one is the model he chose
//! (the tower's `Settings`); tests put a stand-in in its place.
use super::Boxed;
use serde_json::Value;

/// A model asked one question. Err says why there is no answer, in the words of this computer's log.
pub(in crate::tower) trait Model: Sync {
    /// Ask, and when `shape` is given, hold the answer to that JSON schema where the service can be.
    fn ask<'a>(&'a self, prompt: String, shape: Option<Value>) -> Boxed<'a, Result<String, String>>;
}

/// A model that always gives the same answer, and counts how often it was asked.
#[cfg(test)]
pub(in crate::tower) struct FakeModel {
    pub(in crate::tower) answer: Result<String, String>,
    pub(in crate::tower) asked: std::sync::atomic::AtomicUsize,
    /// Every question it was put, in order.
    pub(in crate::tower) heard: std::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl FakeModel {
    pub(in crate::tower) fn answers(answer: Result<&str, &str>) -> FakeModel {
        FakeModel { answer: answer.map(str::to_string).map_err(str::to_string), asked: Default::default(), heard: Default::default() }
    }
}

#[cfg(test)]
impl Model for FakeModel {
    fn ask<'a>(&'a self, prompt: String, _shape: Option<Value>) -> Boxed<'a, Result<String, String>> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.heard.lock().unwrap().push(prompt);
        let answer = self.answer.clone();
        Box::pin(async move { answer })
    }
}
