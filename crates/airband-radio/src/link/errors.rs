//! What went wrong reaching the tower, and whether to try again.
use super::*;

/// What went wrong with one request. Each is a different thing to tell him and a different thing to do next;
/// none is folded into another.
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum LinkError {
    /// Nothing answered that way: no network, or nothing at that address.
    Unreachable(Way),
    /// It was reached and gave no answer in the time allowed.
    TimedOut(Way),
    /// The relay is there and says his computer is not connected to it: asleep, off or offline. What he says
    /// can be left with the relay for it.
    Away,
    /// His computer is back and still being handed what was left for it: what he says now joins the end of that.
    Behind,
    /// The relay handed the request to his computer and no answer came back in the time the relay waits.
    TowerSilent,
    /// The relay answered, and not with his computer's answer.
    Relay(u16),
    /// The relay would not keep what he said.
    HoldRefused(u16),
    /// An answer came that this pairing's key does not open, or that was to another request.
    Unreadable(Way),
    /// His computer answered, and the answer was no, as it said it ([`Refusal`]).
    Refused(Refusal),
    /// His computer answered with a status that carried neither its answer nor a refusal: trouble of its own
    /// (5xx), which may pass, or a no it gave no reason for.
    Trouble(u16),
    /// His computer took something he said and would not act on it, for the reason it gave.
    Rejected(airband_core::radio::Rejected),
    /// The relay says this pairing was ended: nothing reaches that computer this way until he pairs again.
    Ended,
    /// The relay does not know this phone for that computer, and it did once: the pairing was taken away
    /// there, and nothing reaches it this way until he pairs again.
    Forgotten,
}

/// What he is told when his computer speaks another version of the radio protocol. No capital, no full stop.
pub(crate) const OTHER_VERSION: &str = "this computer runs another version of airband: update it and this app";

/// What to do after a failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Retry {
    /// It may well work in a moment.
    Soon,
    /// It will not work for a while: something is full, or was asked too often.
    Later,
    /// It will not work however often it is tried: he has to do something, or it cannot be sent.
    Never,
}

impl LinkError {
    pub(crate) fn retry(&self) -> Retry {
        match self {
            LinkError::Unreachable(_) | LinkError::TimedOut(_) | LinkError::Away | LinkError::Behind | LinkError::TowerSilent => Retry::Soon,
            // more than the relay carries, or not something it keeps: the same again gets the same answer
            LinkError::Relay(413) | LinkError::HoldRefused(400 | 413) => Retry::Never,
            // The relay does not know this phone for that computer. Just after pairing that is so for a few
            // seconds, until the computer has opened its side; the phone cannot tell that from a pairing that
            // was taken away, so it keeps trying and says what the relay said.
            LinkError::Relay(401 | 403) | LinkError::HoldRefused(401 | 403) => Retry::Soon,
            LinkError::Relay(500..=599) | LinkError::HoldRefused(500..=506 | 508..=599) => Retry::Soon,
            LinkError::Relay(_) | LinkError::HoldRefused(_) | LinkError::Unreadable(_) => Retry::Later,
            // the computer is in trouble of its own, which may pass; anything else it refuses, it refuses for good
            LinkError::Trouble(500..=599) | LinkError::Refused(Refusal::NotNow { .. }) => Retry::Later,
            LinkError::Refused(_) | LinkError::Trouble(_) | LinkError::Rejected(_) | LinkError::Ended | LinkError::Forgotten => Retry::Never,
        }
    }
    /// What the relay's status says to a radio on one of its two routes, read in this one place: Ok when it
    /// passed the request on (a call answered, a message kept), else what it said.
    pub(crate) fn relay_said(route: airband_core::relay::Route, status: u16) -> Result<(), LinkError> {
        use airband_core::relay::Route;
        match (route, status) {
            (Route::Call, 200) | (Route::Queue, 202) => Ok(()),
            (_, 410) => Err(LinkError::Ended),
            (Route::Call, 409) => Err(LinkError::Behind),
            (Route::Call, 503) => Err(LinkError::Away),
            (Route::Call, 504) => Err(LinkError::TowerSilent),
            (Route::Queue, status) => Err(LinkError::HoldRefused(status)),
            (_, status) => Err(LinkError::Relay(status)),
        }
    }
    /// The same, for a relay that has passed on a request of this phone's before (`knew`): its not knowing
    /// the phone now is a pairing taken away, not one still being opened.
    pub(crate) fn relay_said_after(route: airband_core::relay::Route, status: u16, knew: bool) -> Result<(), LinkError> {
        match Self::relay_said(route, status) {
            Err(LinkError::Relay(401 | 403) | LinkError::HoldRefused(401 | 403)) if knew => Err(LinkError::Forgotten),
            said => said,
        }
    }
    /// What a status other than 200 from his computer comes to: the refusal its body carries, read by its tag
    /// (never by the status), or, when it carries none, its trouble.
    pub(crate) fn refusal(status: u16, body: Value) -> LinkError {
        match serde_json::from_value::<Refusal>(body) {
            Ok(refusal) => LinkError::Refused(refusal),
            Err(_) => LinkError::Trouble(status),
        }
    }
    /// What happened, as a sentence of its own.
    pub(crate) fn said(&self) -> String {
        format!("{}.", airband_core::prose::capital(&self.told()))
    }
    /// What happened, in words for him. No capital and no full stop: it is joined to others.
    pub(crate) fn told(&self) -> String {
        match self {
            LinkError::Unreachable(Way::Relay | Way::Kept) => "there was no connection to the relay".into(),
            LinkError::Unreachable(Way::Near) => "your computer did not answer on this network".into(),
            LinkError::Unreachable(Way::Paired) => "there was no connection to your computer".into(),
            LinkError::TimedOut(Way::Relay | Way::Kept) => "the relay did not answer in time".into(),
            LinkError::TimedOut(Way::Near | Way::Paired) => "your computer did not answer in time".into(),
            LinkError::Away => "the relay says your computer is not connected".into(),
            LinkError::Behind => "earlier messages are still on their way to your computer".into(),
            LinkError::TowerSilent => "your computer took it from the relay and has not answered yet".into(),
            LinkError::Relay(401 | 403) | LinkError::HoldRefused(401 | 403) => "the relay does not know this phone for that computer".into(),
            LinkError::Relay(413) | LinkError::HoldRefused(413) => "it is more than the relay carries in one message".into(),
            LinkError::Relay(429) | LinkError::HoldRefused(429) => "the relay was being asked too often".into(),
            LinkError::Relay(status) => format!("the relay answered {status}"),
            LinkError::HoldRefused(507) => "the relay is keeping as many messages as it can for your computer".into(),
            LinkError::HoldRefused(status) => format!("the relay could not keep it ({status})"),
            LinkError::Unreadable(Way::Near | Way::Paired) => "your computer sent an answer that could not be read".into(),
            LinkError::Unreadable(_) => "an answer came through the relay that could not be read".into(),
            LinkError::Refused(Refusal::Told { reason } | Refusal::BadAsk { reason }) => format!("your computer refused it: {reason}"),
            LinkError::Refused(Refusal::OtherVersion { .. }) => OTHER_VERSION.into(),
            LinkError::Refused(Refusal::NotNow { reason }) => format!("your computer could not do it just now: {reason}"),
            LinkError::Refused(Refusal::TooBig) => "it is more than your computer takes in one message".into(),
            LinkError::Refused(Refusal::Gone) => "your computer no longer has what it names".into(),
            LinkError::Refused(Refusal::NotAllowed) => "your computer does not let this phone do that".into(),
            LinkError::Refused(Refusal::NotPaired) => "your computer no longer knows this phone; pair it again".into(),
            LinkError::Refused(Refusal::Unknown) => "your computer refused it".into(),
            // trouble of its own, which may pass: not a no
            LinkError::Trouble(status @ 500..=599) => format!("your computer ran into trouble with it ({status})"),
            LinkError::Trouble(status) => format!("your computer refused it ({status})"),
            LinkError::Rejected(airband_core::radio::Rejected::TooOld) => "your computer got it more than a week after you said it, and did nothing with it".into(),
            LinkError::Rejected(airband_core::radio::Rejected::Unknown) => "your computer would not take it".into(),
            LinkError::Ended => "the relay says this pairing was ended; pair this phone again".into(),
            LinkError::Forgotten => "the relay no longer knows this phone for that computer; pair this phone again".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_will_never_go_is_told_as_such_and_not_as_waiting_for_a_connection() {
        let e: RadioError = LinkError::Relay(413).into();
        assert!(matches!(&e, RadioError::Told { reason } if reason == "It is more than the relay carries in one message."), "{e:?}");
        assert!(matches!(RadioError::from(LinkError::Away), RadioError::Offline));
    }

    #[test]
    fn the_relay_is_read_by_route_and_a_pairing_it_ended_is_told_as_that() {
        use airband_core::relay::Route;
        assert_eq!(LinkError::relay_said(Route::Call, 200), Ok(()));
        assert_eq!(LinkError::relay_said(Route::Queue, 202), Ok(()));
        assert_eq!(LinkError::relay_said(Route::Call, 503), Err(LinkError::Away));
        assert_eq!(LinkError::relay_said(Route::Queue, 507), Err(LinkError::HoldRefused(507)));
        // the relay's 410 on either route is a pairing that was ended: he is told to pair again (audit 8)
        for route in [Route::Call, Route::Queue] {
            assert_eq!(LinkError::relay_said(route, 410), Err(LinkError::Ended));
        }
        assert_eq!(LinkError::Ended.retry(), Retry::Never);
        assert!(LinkError::Ended.said().contains("pair this phone again"));
        // (audit 9, B-41) not knowing this phone is a pairing still being opened only until the relay has
        // once passed on a request of its: after that it is one taken away, and he is told to pair again
        assert_eq!(LinkError::relay_said_after(Route::Call, 401, false), Err(LinkError::Relay(401)));
        assert_eq!(LinkError::Relay(401).retry(), Retry::Soon);
        for route in [Route::Call, Route::Queue] {
            assert_eq!(LinkError::relay_said_after(route, 401, true), Err(LinkError::Forgotten));
            assert_eq!(LinkError::relay_said_after(route, 403, true), Err(LinkError::Forgotten));
        }
        assert_eq!(LinkError::relay_said_after(Route::Call, 503, true), Err(LinkError::Away));
        assert_eq!(LinkError::Forgotten.retry(), Retry::Never);
        assert!(LinkError::Forgotten.said().ends_with("pair this phone again."));
    }
}
