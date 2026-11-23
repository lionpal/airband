//! What the next thing he says carries with it, kept beside the link and not in it: the files he attached that
//! have reached his computer, and the question his computer last asked him back.
use super::*;
use std::collections::VecDeque;

/// How many questions already asked are remembered, so one told again (the feed can tell an answer more than
/// once) is not asked a second time.
const ASKED_KEEP: usize = 64;

#[derive(Default)]
pub(crate) struct Session {
    /// Files he has attached and that have reached the tower: the next thing he says takes them with it.
    armed: Mutex<Vec<String>>,
    /// The question his computer last asked him back, by its id, until the next thing he says to it, which
    /// carries it as its answer. Not kept across restarts.
    question: Mutex<Option<String>>,
    /// The questions already asked, newest last.
    asked: Mutex<VecDeque<String>>,
}

impl Session {
    pub(crate) fn arm(&self, id: String) {
        let mut armed = self.armed.lock().unwrap();
        if !armed.contains(&id) {
            armed.push(id);
        }
    }
    pub(crate) fn disarm(&self, id: &str) {
        self.armed.lock().unwrap().retain(|a| a != id);
    }
    pub(crate) fn armed(&self) -> Vec<String> {
        self.armed.lock().unwrap().clone()
    }
    pub(crate) fn take_armed(&self) -> Vec<String> {
        std::mem::take(&mut *self.armed.lock().unwrap())
    }
    /// What something that did not go took with it is his again: the files, and the question it answered
    /// when nothing newer has been asked since.
    pub(crate) fn put_back(&self, files: Vec<String>, question: Option<String>) {
        files.into_iter().for_each(|id| self.arm(id));
        if let Some(q) = question {
            self.question.lock().unwrap().get_or_insert(q);
        }
    }

    /// His computer asked him something back: what he says next answers it. A question already asked is not
    /// asked again when the same answer is told a second time.
    pub(crate) fn asked(&self, reply: &Reply) {
        let Some(question) = reply.question.as_ref().map(|q| q.as_str().to_string()) else { return };
        let mut asked = self.asked.lock().unwrap();
        if asked.contains(&question) {
            return;
        }
        asked.push_back(question.clone());
        if asked.len() > ASKED_KEEP {
            asked.pop_front();
        }
        *self.question.lock().unwrap() = Some(question.clone());
    }
    /// The question his next words answer, taken: only the next thing he says carries it.
    pub(crate) fn take_question(&self) -> Option<String> {
        self.question.lock().unwrap().take()
    }

    /// Something he said aloud, sent now and waited for ([`Link::say_and_wait`]). It takes the files he
    /// attached and answers the question he was asked; if it never got there, both are his again.
    pub(crate) fn say(&self, link: &Link, text: &str, re: Option<String>, scope: Option<String>, act: Option<Act>) -> Result<Option<Reply>, LinkError> {
        let attachments = self.take_armed();
        let answers = self.take_question();
        let out = Outgoing { answers: answers.clone(), ..Outgoing::new(random_hex(12), text.to_string(), re, scope, None, attachments.clone(), Vec::new()).with_act(act.as_ref()) };
        let u = out.utterance();
        let got = link.say_and_wait(&u);
        match &got {
            Ok(Some(reply)) => self.asked(reply),
            Ok(None) => {}
            Err(_) => self.put_back(attachments, answers),
        }
        got
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asking(id: &str) -> Reply {
        Reply { question: Some(airband_core::ids::QuestionId::new(id)), ..Reply::default() }
    }

    #[test]
    fn an_answer_told_again_does_not_ask_its_question_again() {
        let session = Session::default();
        session.asked(&asking("q1"));
        assert_eq!(session.take_question().as_deref(), Some("q1"));
        // the feed tells the same answer a second time: he has already answered it
        session.asked(&asking("q1"));
        assert_eq!(session.take_question(), None);
    }

    #[test]
    fn what_he_said_aloud_that_did_not_go_leaves_him_the_question_and_the_files() {
        let session = Session::default();
        session.asked(&asking("q1"));
        session.arm("f1".into());
        // nobody is at this address
        let link = Link::new(airband_core::pairing::Pairing::parse("airband://pair?url=http%3A%2F%2F127.0.0.1%3A9&token=t&name=Mac").unwrap(), "test", crate::link::Trail::open(None)).unwrap();
        assert!(session.say(&link, "yes", None, None, None).is_err());
        assert_eq!(session.take_question().as_deref(), Some("q1"));
        assert_eq!(session.armed(), ["f1"]);
    }
}
