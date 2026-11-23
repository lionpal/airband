//! The ids everything is keyed by, one type each, so a team's id can never be passed where an agent's is wanted
//! and nothing is ever keyed by a name or a place in a list. On the wire each is the plain string it wraps.

use serde::{Deserialize, Serialize};

macro_rules! ids {
    ($($(#[$doc:meta])* $name:ident;)*) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// The id a tool, a row or the wire gave, as it was given: the one way one is made.
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
            pub fn into_string(self) -> String {
                self.0
            }
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    )*};
}

ids! {
    /// One agent's session: the id its tool gives it, unique on its computer.
    AgentId;
    /// One of his teams on one computer.
    TeamId;
    /// One computer (a tower), as its pairing names it: the same on every phone it is paired with.
    StationId;
    /// One way of starting an agent (a launcher row).
    LauncherId;
    /// One question the tower put to him and waits on.
    QuestionId;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_its_plain_string_on_the_wire() {
        let id = AgentId::new("abc");
        assert_eq!(serde_json::to_string(&id).unwrap(), r#""abc""#);
        assert_eq!(serde_json::from_str::<AgentId>(r#""abc""#).unwrap(), id);
        let mut by: std::collections::BTreeMap<TeamId, u8> = Default::default();
        by.insert(TeamId::new("t"), 1);
        assert_eq!(by.get(&TeamId::new("t")), Some(&1));
        assert_eq!(id.to_string(), "abc");
    }
}
