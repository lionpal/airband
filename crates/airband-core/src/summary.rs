//! What the model that writes his summaries is told about how to write them: the part he can change.
//!
//! The question a computer puts to that model has three parts. The first says what the job is and the kinds a
//! message can be; the last names every field of the answer and the shape it is read in. Both are the
//! computer's own and fixed, so nothing he writes can break how the answer is read. Between them is how to
//! write what he is told: that part is his. Until he changes it, it is [`USUAL`]; he edits it on his phone and
//! every computer is sent his text; Reset gives him [`USUAL`] back.

/// How to write what he is told, as it stands until he changes it.
pub const USUAL: &str = "Then what to tell him, in plain spoken English. He will not read much: he glances. One short sentence, \
14 words at most. If the agent is asking him something, the sentence is the question itself, as \
briefly as it can be put (\"Two questions are too easy. Rewrite or leave?\"); how it got there is \
one tap away and does not belong here.
The one exception is findings he asked for, so check for it before writing anything: look at the last \
thing he told this agent, given below. If that was to investigate, research, review, compare, audit or \
explain something, and this message is the answer, a headline would throw the answer away. Then \"say\" \
is an executive summary instead: three to five short sentences, between 40 and 90 words, the conclusion \
first, then the findings that matter with their numbers as digits, then what he has to decide if \
anything. It is read aloud to him, so it is sentences, never a list.
The same longer form is for a message that leaves him several things to decide or do: he cannot act on \
a count. Never say that there are some number of things (\"four decisions needed\", \"three issues found\") \
without saying what each one is, in a few words each: \"Before going public: delete the old logs, pick a \
commit email, decide on the account code.\" If naming them takes more than one sentence, take the room.
- Keep only: the result; anything left undone or gone wrong; the one thing he must decide.
- Always keep anything the work is waiting on that only he can do (a request he has to file, an approval, \
a payment, a sign-in, a setting only his account can change, a choice he has not made), whether or not \
the agent has stopped for it: an agent that went on with other work has still left that part blocked on \
him. Say it in its own sentence, beginning \"Waiting on you:\", with what he has to do and what it \
unblocks (\"Waiting on you: approve the payment provider's form, before refunds can go out.\"). \
If there are several, name each. This comes before detail about what was done.
- Always keep what the work left outside the conversation, each with the number or name that finds it: \
a push or commit (its short id), a pull request or ticket (its number), a build or release (its number or \
version, and where it went), a deploy (to what), a published page or file, a message sent to someone. \
These are what he will go and look for, so they outrank detail about how the work was done. Say plainly \
when one of them was made but not finished (built but not uploaded, committed but not pushed).
- If the agent needs an answer from him, end with that question, with the options if it gave any. \
The question does not stand in for the rest: a message that reports finished work and then asks \
something is told as the news first (what is now done, live, moved, spent or broken) and the question \
last. He must not lose the news because a question came after it. Each finished thing is news of its own: \
\"the bug is fixed, and the refunds are ready to send\" keeps both, and does not shrink to the one the \
question is about. Take the longer form for it.
- No file paths, code, lists or markdown, and no identifiers other than those just named. Say \"the checker script\", not its filename.
- Speak as the agent, in the first person. No greeting, no padding (\"nothing needed from you\", \"let me know\").
- Say only what the message says. Add no fact, date or reassurance that is not in it.";

/// The most his text can be, in characters: twice the usual text and more, room to rewrite it whole, and a
/// bound on what every summary sends.
pub const MOST: usize = 8000;

/// Why his text was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// There is nothing in it.
    Empty,
    /// It is longer than [`MOST`].
    TooLong,
}

impl Refused {
    /// In words for him.
    pub fn said(self) -> String {
        match self {
            Refused::Empty => "There's nothing to save. Reset brings back the usual text.".into(),
            Refused::TooLong => format!("That's too long. Keep it under {MOST} characters."),
        }
    }
}

/// His text as it is to be kept: nothing when it is the usual text (so a later change to the usual reaches
/// him), else his own words, trimmed at the ends. Err when it cannot be used.
pub fn checked(text: &str) -> Result<Option<String>, Refused> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Refused::Empty);
    }
    if text.chars().count() > MOST {
        return Err(Refused::TooLong);
    }
    Ok((text != USUAL).then(|| text.to_string()))
}

/// How to write what he is told: his text when he has one, else the usual.
pub fn how(his: Option<&str>) -> &str {
    his.unwrap_or(USUAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn his_text_is_kept_trimmed_and_the_usual_or_nothing_is_not_his() {
        assert_eq!(checked("  Call me sir.\n"), Ok(Some("Call me sir.".into())));
        assert_eq!(checked(USUAL), Ok(None));
        assert_eq!(checked(&format!("\n{USUAL}  ")), Ok(None));
        assert_eq!(checked(" \n "), Err(Refused::Empty));
        assert_eq!(checked(&"a".repeat(MOST)), Ok(Some("a".repeat(MOST))));
        assert_eq!(checked(&"é".repeat(MOST + 1)), Err(Refused::TooLong));
        assert!(USUAL.chars().count() < MOST / 2);
        assert_eq!(how(None), USUAL);
        assert_eq!(how(Some("x")), "x");
    }
}
