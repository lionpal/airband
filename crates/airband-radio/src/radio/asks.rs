//! The radio's plain requests to its computer: one request each, answered or refused, for the desk and the
//! command line. Nothing here keeps state.
use super::*;

impl Radio {
    /// The rest of a message: the fuller spoken version, or with `full` exactly what the agent wrote.
    pub(crate) fn more(&self, id: String, full: bool) -> Result<String, RadioError> {
        #[derive(serde::Deserialize)]
        struct Whole {
            text: String,
        }
        Ok(self.asked::<Whole>(Method::Get, &format!("/message/{id}?{}", airband_core::radio::MessageAsk { form: Some(if full { airband_core::radio::Form::Full } else { airband_core::radio::Form::Detail }), part: None }.query()), None)?.text)
    }

    /// The accounts the agents on this computer can run under, and how much of each is used. `fresh` asks the
    /// tower to check again now. Blocks: call it off the main thread.
    pub(crate) fn accounts(&self, computer: &crate::DeskComputer, fresh: bool) -> Result<StationAccounts, RadioError> {
        Ok(StationAccounts::from(computer, self.asked(Method::Get, if fresh { "/accounts?fresh=1" } else { "/accounts" }, None)?))
    }

    /// Move this computer's sessions of one harness to another account. Returns how things stand afterwards.
    /// Blocks: call it off the main thread.
    pub(crate) fn use_account(&self, computer: &crate::DeskComputer, harness: String, id: String) -> Result<StationAccounts, RadioError> {
        Ok(StationAccounts::from(computer, self.asked(Method::Post, "/accounts/use", Some(serde_json::to_value(airband_core::radio::UseAccount { harness, id }).unwrap()))?))
    }

    /// Put an agent in another team: one that exists, or a new one of that name. Returns what the agent is
    /// called now and the team it is in. Blocks: call it off the main thread.
    pub(crate) fn move_agent(&self, agent: String, to: airband_core::radio::TeamPick) -> Result<airband_core::radio::Moved, RadioError> {
        self.asked(Method::Post, "/team", Some(serde_json::to_value(airband_core::radio::MoveAgent { agent, to }).unwrap()))
    }

    /// Stop what an agent the tower runs is doing, now. Blocks: call it off the main thread.
    pub(crate) fn stop_agent(&self, agent: String) -> Result<(), RadioError> {
        self.ask("/stop", &airband_core::radio::StopAgent { agent })
    }
    /// End an agent the tower runs. Blocks: call it off the main thread.
    pub(crate) fn end_agent(&self, agent: String) -> Result<(), RadioError> {
        self.ask("/end", &airband_core::radio::EndAgent { agent })
    }
    /// Tell this computer which names his other computers' agents go by. Blocks.
    pub(crate) fn taken(&self, callsigns: &[airband_core::routing::Callsign]) -> bool {
        let taken = airband_core::radio::Taken { callsigns: callsigns.to_vec() };
        self.link.request(Method::Post, "/taken", Some(serde_json::to_value(taken).unwrap())).is_ok_and(|(status, _)| status == 200)
    }
    /// Let a question go without answering it. Blocks: call it off the main thread.
    pub(crate) fn dismiss(&self, message: String) -> Result<(), RadioError> {
        self.ask("/dismiss", &airband_core::radio::Dismiss { message })
    }
    /// Take back something he said that is still waiting its turn. Blocks: call it off the main thread.
    /// Choose what reads for this tower. Blocks while the tower tries it with a real call.
    pub(crate) fn set_reads(&self, wanted: &airband_core::radio::SetReads) -> Result<(), RadioError> {
        self.ask("/reads", wanted)
    }

    /// What this computer can read with, and what it reads with now, by its own account. Nothing when it
    /// cannot be reached, or is a version from before it could say. Blocks.
    pub(crate) fn reads_offered(&self) -> Option<airband_core::radio::ReadsOffered> {
        self.asked(Method::Get, "/reads", None).ok()
    }

    /// The folders inside one of his on this computer. Blocks.
    /// Rename one of his teams on this computer. Answers with the name it now has. Blocks.
    pub(crate) fn rename_team(&self, team: String, to: String) -> Result<String, RadioError> {
        #[derive(serde::Deserialize)]
        struct Renamed {
            team: String,
        }
        Ok(self.asked::<Renamed>(Method::Post, "/team/rename", Some(serde_json::to_value(airband_core::radio::RenameTeam { team, to }).unwrap()))?.team)
    }
    pub(crate) fn folders(&self, under: String) -> Result<airband_core::radio::Folders, RadioError> {
        self.asked(Method::Post, "/folders", Some(serde_json::to_value(airband_core::radio::FoldersAsk { under }).unwrap()))
    }
    /// The folders the coding tools on this computer have had sessions in, newest first. Blocks.
    pub(crate) fn recent(&self) -> Result<airband_core::radio::Recent, RadioError> {
        self.asked(Method::Get, "/recent", None)
    }
    /// Make a new team on this computer, or start another agent in one that is there. Blocks.
    /// Answers with the team's id on the computer.
    pub(crate) fn new_project(&self, name: String, folder: String, make: bool, tool: Option<String>) -> Result<String, RadioError> {
        #[derive(serde::Deserialize)]
        struct Made {
            id: String,
        }
        Ok(self.asked::<Made>(Method::Post, "/project", Some(serde_json::to_value(airband_core::radio::NewProject { name, folder, make, tool }).unwrap()))?.id)
    }

    pub(crate) fn unsend(&self, message: String) -> Result<(), RadioError> {
        self.ask("/unsend", &airband_core::radio::Unsend { message })
    }

    /// Have the tower take over an agent he started in a terminal: that session ends there and carries on in a
    /// process the tower holds. Blocks: call it off the main thread.
    pub(crate) fn take_over(&self, agent: String) -> Result<(), RadioError> {
        self.ask("/takeover", &airband_core::radio::TakeOver { agent })
    }

    /// Tell the tower which coding tools he uses, the one he uses most first.
    pub(crate) fn uses_tools(&self, tools: &[String]) -> Result<(), RadioError> {
        self.ask("/tools", &airband_core::radio::UsesTools { tools: tools.to_vec() })
    }

    /// Tell the tower where this radio can be woken with a notification (its address at a push service), or
    /// with an empty token that it no longer can.
    pub(crate) fn push_target(&self, provider: desk::PushService, token: &str) -> Result<(), RadioError> {
        self.ask("/push", &airband_core::radio::PushTarget { provider, token: token.into() })
    }
}
