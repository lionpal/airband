//! Where on a relay each of a channel's routes is.

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
    /// Its path for this channel.
    pub fn on(self, channel: &str) -> String {
        let tail = match self {
            Route::Channel => String::new(),
            Route::Pull => "/pull".into(),
            Route::Reply(id) => format!("/reply/{}", crate::encode::address_part(id)),
            Route::Call => "/call".into(),
            Route::Queue => "/queue".into(),
            Route::Push => "/push".into(),
        };
        format!("/v1/channels/{channel}{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::{Keys, new_secret};

    #[test]
    fn each_relay_route_is_reached_where_the_relay_serves_it() {
        let keys = Keys::derive(&new_secret()).unwrap();
        for (pattern, route) in Route::PATTERNS {
            let reached = keys.to("https://relay.test/", route);
            let served = pattern.replace("{channel}", &keys.channel).replace("{id}", "%7Bid%7D");
            assert_eq!(reached, format!("https://relay.test{served}"));
            assert_eq!(route.pattern(), pattern);
        }
        assert!(keys.to("r", Route::Reply("a/b")).ends_with("/reply/a%2Fb"), "an id is one segment");
    }
}
