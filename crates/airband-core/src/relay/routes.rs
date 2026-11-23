//! Where on a relay each of a channel's routes is, and the one route a tower serves on his own network.

/// The route a tower serves on his own network, as its router writes it: the same sealed request a radio would
/// give the relay, answered with the sealed answer ([`crate::relay::Keys::straight`] for a channel's address).
pub const SEALED: &str = "/v1/sealed/{channel}";

/// The relay's routes for one channel: each written once, as the relay serves it ([`Route::PATTERNS`]) and as a
/// tower or a radio reaches it ([`Route::on`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route<'a> {
    /// `PUT` opens the channel (its tower), `DELETE` closes it.
    Channel,
    /// `GET`: the tower waits here for what its radios sent.
    Pull,
    /// `POST`: the tower's answer to one request.
    Reply(&'a str),
    /// `POST`: a radio's request, answered when the tower answers.
    Call,
    /// `POST`: a radio's request kept until the tower comes for it.
    Queue,
    /// `POST`: the tower asks for the phone to be woken.
    Push,
}

impl Route<'_> {
    /// Every route as the relay's router writes it, with `{channel}` and `{id}` where they go.
    pub const PATTERNS: [(&'static str, Route<'static>); 6] = [
        ("/v1/channels/{channel}", Route::Channel),
        ("/v1/channels/{channel}/pull", Route::Pull),
        ("/v1/channels/{channel}/reply/{id}", Route::Reply("{id}")),
        ("/v1/channels/{channel}/call", Route::Call),
        ("/v1/channels/{channel}/queue", Route::Queue),
        ("/v1/channels/{channel}/push", Route::Push),
    ];
    /// The route's pattern for the relay's router.
    pub fn pattern(self) -> &'static str {
        Route::PATTERNS.iter().find(|(_, r)| std::mem::discriminant(r) == std::mem::discriminant(&self)).map(|(p, _)| *p).unwrap_or_default()
    }
    /// Its path for this channel: its pattern ([`Route::PATTERNS`]) filled in, so each path is written once.
    pub fn on(self, channel: &str) -> String {
        let id = match self {
            Route::Reply(id) => crate::encode::address_part(id),
            _ => String::new(),
        };
        crate::template::filled(self.pattern(), &[("channel", channel), ("id", &id)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::{Keys, new_secret};

    #[test]
    fn each_relay_route_is_reached_where_the_relay_serves_it() {
        let keys = Keys::derive(&new_secret()).unwrap();
        let c = &keys.channel;
        let want = [format!("/v1/channels/{c}"), format!("/v1/channels/{c}/pull"), format!("/v1/channels/{c}/reply/%7Bid%7D"), format!("/v1/channels/{c}/call"), format!("/v1/channels/{c}/queue"), format!("/v1/channels/{c}/push")];
        for ((pattern, route), want) in Route::PATTERNS.into_iter().zip(want) {
            assert_eq!(keys.to("https://relay.test/", route), format!("https://relay.test{want}"));
            assert_eq!(route.pattern(), pattern);
        }
        assert!(keys.to("r", Route::Reply("a/b")).ends_with("/reply/a%2Fb"), "an id is one segment");
        // a channel's name is never read again as a place
        assert_eq!(Route::Pull.on("{id}"), "/v1/channels/{id}/pull");
        // the tower's own route, reached where its router serves it
        assert_eq!(keys.straight("http://192.168.1.4:7400/"), format!("http://192.168.1.4:7400/v1/sealed/{c}"));
    }
}
