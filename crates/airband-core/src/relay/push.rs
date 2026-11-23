//! Waking his phone through the relay: the tower's request, the relay's answer, and why it was not sent.
use serde::{Deserialize, Serialize};

/// A push service a phone can be woken through.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum PushService {
    /// Apple's.
    #[serde(rename = "apns")]
    Apns,
    /// One a newer build knows and this one does not: kept, and never sent to.
    #[default]
    #[serde(other, rename = "unknown")]
    Unknown,
}

/// `POST /v1/channels/<channel>/push`, from the channel's tower: have a push service wake the phone. The relay
/// passes `sealed` on as it is and can read none of it; the phone opens it with the pairing's key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRequest {
    /// Which push service the address belongs to.
    pub provider: PushService,
    /// The phone's address at that service, as the phone gave it to the tower.
    pub to: String,
    /// A notification sealed for [`Direction::Push`], base64.
    pub sealed: String,
    /// It is a question to him. All the relay and the push service learn about what is inside.
    #[serde(default)]
    pub question: bool,
}

/// The most characters a phone's address at a push service has.
const PUSH_ADDRESS_MOST: usize = 256;

/// Is this the shape of a phone's address at a push service: a short run of letters, digits, `-`, `_` and `:`?
/// The one rule, for the tower taking an address from a radio and the relay passing one on.
pub fn push_address(to: &str) -> bool {
    !to.is_empty() && to.len() <= PUSH_ADDRESS_MOST && to.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
}

impl PushRequest {
    /// Is it the right shape to pass on: an address ([`push_address`]), and a sealed notification that is
    /// base64 of no more than [`PUSH_MOST`] bytes?
    pub fn well_formed(&self) -> bool {
        let base64 = !self.sealed.is_empty() && self.sealed.bytes().all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b));
        push_address(&self.to) && base64 && self.sealed.len() <= PUSH_MOST.div_ceil(3) * 4
    }
}

/// The relay's answer to a [`PushRequest`]. The HTTP status says the same in a number: 200 sent, 400 a request
/// that is not one, 410 the address is no longer good, 429 too many, 503 this relay has no such push service, 502
/// the service refused or failed. A 403 or 404 (not this channel's tower, no such channel) carries no
/// `PushAnswer`: those are the relay's answers to any tower route.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PushAnswer {
    pub sent: bool,
    /// The phone no longer takes notifications at that address: the tower forgets it.
    #[serde(default)]
    pub gone: bool,
    /// Why not, as a fact a tower can act on; none when it was sent. The push service's own words stay in the
    /// relay's log.
    #[serde(default)]
    pub why: Option<NotPushed>,
}

impl PushAnswer {
    /// Not sent, for this reason.
    pub fn not_sent(why: NotPushed) -> PushAnswer {
        PushAnswer { sent: false, gone: why == NotPushed::Gone, why: Some(why) }
    }
}

/// Why the relay did not hand a notification on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotPushed {
    /// Too many from the tower's internet address lately.
    TooManyFromAddress,
    /// Too many on this channel lately.
    TooManyOnChannel,
    /// The body was not a [`PushRequest`].
    NotARequest,
    /// The address or the sealed notification was not the right shape ([`PushRequest::well_formed`]).
    WrongShape,
    /// This relay has no such push service.
    NoSuchService,
    /// The phone no longer takes notifications at that address.
    Gone,
    /// The push service said no, with this status.
    Refused { status: u16 },
    /// The relay could not sign what the push service asks for.
    CannotSign,
    /// The push service did not answer in time.
    TimedOut,
    /// The push service could not be reached.
    Unreachable,
    /// A reason a newer relay has and this build does not.
    #[serde(other)]
    Other,
}

impl std::fmt::Display for NotPushed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotPushed::TooManyFromAddress => f.write_str("too many notifications from this address; try again later"),
            NotPushed::TooManyOnChannel => f.write_str("too many notifications from this channel; try again later"),
            NotPushed::NotARequest => f.write_str("not a push request"),
            NotPushed::WrongShape => f.write_str("the address or the sealed notification is not the right shape"),
            NotPushed::NoSuchService => f.write_str("this relay has no such push service"),
            NotPushed::Gone => f.write_str("the phone's address is no longer good"),
            NotPushed::Refused { status } => write!(f, "the push service said no (status {status})"),
            NotPushed::CannotSign => f.write_str("the push key could not sign"),
            NotPushed::TimedOut => f.write_str("the push service did not answer in time"),
            NotPushed::Unreachable => f.write_str("the push service could not be reached"),
            NotPushed::Other => f.write_str("the relay did not send it"),
        }
    }
}

/// The largest sealed notification, in bytes before base64: what fits in one push with room for the rest.
pub const PUSH_MOST: usize = 2400;
