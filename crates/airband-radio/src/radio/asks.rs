//! The radio's plain requests to its computer: one request each, answered or refused, for the desk and the
//! command line. Nothing here keeps state.
use super::*;

impl Radio {
    /// A request whose answer is nothing but that it was done.
    fn done<E: wire::Endpoint<Query = Nothing, Answer = wire::Done>>(&self, body: &E::Body) -> Result<(), RadioError> {
        self.link.call::<E>(None, &Nothing {}, body)?;
        Ok(())
    }

    /// The rest of a message: the fuller spoken version, or with `full` exactly what the agent wrote.
    pub(crate) fn more(&self, id: String, full: bool) -> Result<String, RadioError> {
        let form = Some(if full { airband_core::radio::Form::Full } else { airband_core::radio::Form::Detail });
        Ok(self.link.call::<at::Message>(Some(&id), &wire::MessageQuery { form, part: None }, &Nothing {})?.text)
    }

    /// The accounts the agents on this computer can run under, and how much of each is used. `fresh` asks the
    /// tower to check again now. Blocks: call it off the main thread.
    pub(crate) fn accounts(&self, fresh: bool) -> Result<airband_core::radio::Accounts, RadioError> {
        Ok(self.link.call::<at::Accounts>(None, &wire::AccountsQuery { fresh }, &Nothing {})?)
    }

    /// Move this computer's sessions of one harness to another account. Returns how things stand afterwards.
    /// Blocks: call it off the main thread.
    pub(crate) fn use_account(&self, computer: &crate::DeskComputer, harness: String, id: String) -> Result<StationAccounts, RadioError> {
        Ok(StationAccounts::from(computer, self.link.call::<at::UseAccount>(None, &Nothing {}, &airband_core::radio::UseAccount { harness, id })?))
    }

    /// Put an agent in another team: one that exists, or a new one of that name. Returns what the agent is
    /// called now and the team it is in. Blocks: call it off the main thread.
    pub(crate) fn move_agent(&self, agent: String, to: airband_core::radio::TeamPick) -> Result<airband_core::radio::Moved, RadioError> {
        Ok(self.link.call::<at::Team>(None, &Nothing {}, &airband_core::radio::MoveAgent { agent, to })?)
    }

    /// Stop what an agent the tower runs is doing, now. Blocks: call it off the main thread.
    pub(crate) fn stop_agent(&self, agent: String) -> Result<(), RadioError> {
        self.done::<at::Stop>(&airband_core::radio::StopAgent { agent })
    }
    /// End an agent the tower runs. Blocks: call it off the main thread.
    pub(crate) fn end_agent(&self, agent: String) -> Result<(), RadioError> {
        self.done::<at::End>(&airband_core::radio::EndAgent { agent })
    }
    /// Tell this computer which names his other computers' agents go by. Blocks.
    pub(crate) fn taken(&self, callsigns: &[airband_core::routing::Callsign]) -> bool {
        self.done::<at::Taken>(&airband_core::radio::Taken { callsigns: callsigns.to_vec() }).is_ok()
    }
    /// Let a question go without answering it. Blocks: call it off the main thread.
    pub(crate) fn dismiss(&self, message: String) -> Result<(), RadioError> {
        self.done::<at::Dismiss>(&airband_core::radio::Dismiss { message })
    }
    /// Choose what reads for this tower. Blocks while the tower tries it with a real call.
    pub(crate) fn set_reads(&self, wanted: &airband_core::radio::SetReads) -> Result<(), RadioError> {
        self.link.call::<at::Reads>(None, &Nothing {}, wanted)?;
        Ok(())
    }

    /// What this computer can read with, and what it reads with now, by its own account. Blocks.
    pub(crate) fn reads_offered(&self) -> Result<airband_core::radio::ReadsOffered, RadioError> {
        Ok(self.link.call::<at::ReadsOffered>(None, &Nothing {}, &Nothing {})?)
    }

    /// Rename one of his teams on this computer. Answers with the name it now has. Blocks.
    pub(crate) fn rename_team(&self, team: String, to: String) -> Result<String, RadioError> {
        Ok(self.link.call::<at::RenameTeam>(None, &Nothing {}, &airband_core::radio::RenameTeam { team: team.into(), to })?.team)
    }
    /// The folders inside one of his on this computer. Blocks.
    pub(crate) fn folders(&self, under: String) -> Result<airband_core::radio::Folders, RadioError> {
        Ok(self.link.call::<at::Folders>(None, &Nothing {}, &airband_core::radio::FoldersAsk { under })?)
    }
    /// The folders the coding tools on this computer have had sessions in, newest first. Blocks.
    pub(crate) fn recent(&self) -> Result<airband_core::radio::Recent, RadioError> {
        Ok(self.link.call::<at::Recent>(None, &Nothing {}, &Nothing {})?)
    }
    /// Make a new team on this computer, or start another agent in one that is there. Blocks.
    /// Answers with the team's id on the computer.
    pub(crate) fn new_project(&self, name: String, folder: String, make: bool, tool: Option<String>) -> Result<String, RadioError> {
        Ok(self.link.call::<at::Project>(None, &Nothing {}, &airband_core::radio::NewProject { name, folder, make, tool })?.id)
    }

    /// Take back something he said that is still waiting its turn. Blocks: call it off the main thread.
    pub(crate) fn unsend(&self, message: String) -> Result<(), RadioError> {
        self.done::<at::Unsend>(&airband_core::radio::Unsend { message })
    }

    /// Have the tower take over an agent he started in a terminal: that session ends there and carries on in a
    /// process the tower holds. Blocks: call it off the main thread.
    pub(crate) fn take_over(&self, agent: String) -> Result<(), RadioError> {
        self.done::<at::TakeOver>(&airband_core::radio::TakeOver { agent })
    }

    /// Tell the tower which coding tools he uses, the one he uses most first.
    pub(crate) fn uses_tools(&self, tools: &[String]) -> Result<(), RadioError> {
        self.done::<at::Tools>(&airband_core::radio::UsesTools { tools: tools.to_vec() })
    }

    /// Tell the tower where this radio can be woken with a notification (its address at a push service), or
    /// with an empty token that it no longer can.
    pub(crate) fn push_target(&self, provider: desk::PushService, token: &str) -> Result<(), RadioError> {
        self.done::<at::Push>(&airband_core::radio::PushTarget { provider, token: token.into() })
    }
}
