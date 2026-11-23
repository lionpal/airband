//! Getting his words to an agent. The ways themselves are an edge of the tower (`edges::deliver`); this is
//! the name the rest of the tower calls them by.
use super::*;
use edges::deliver::Deliverer;

/// A new id for one delivery, in the form a tool's own records keep ids in.
pub(super) fn receipt() -> String {
    airband_core::encode::uuid_v4()
}

/// Get `content` to this agent by `how`. `lines` are the chat lines these words are.
pub(super) async fn deliver(app: &Arc<App>, how: &Deliver, agent_id: &str, content: &str, lines: &[String]) -> Result<(), String> {
    match &app.deliverer {
        Some(stand_in) => stand_in.deliver(how, agent_id, content, lines).await,
        None => edges::deliver::AgentWays(app.clone()).deliver(how, agent_id, content, lines).await,
    }
}

#[cfg(test)]
mod seam_tests {
    use super::*;

    #[tokio::test]
    async fn his_words_go_through_the_ways_the_tower_was_given() {
        let ways = Arc::new(edges::deliver::FakeWays::default());
        let app = crate::tower::http::tests::app_on("ways-seam", serde_json::json!({}), Arc::new(edges::procs::FakeProcs::ending_after(Some(0))), Some(ways.clone()));
        deliver(&app, &Deliver::Command { argv: vec!["never-run".into()] }, "a1", "hello", &[]).await.unwrap();
        assert_eq!(ways.0.lock().unwrap().as_slice(), [("a1".to_string(), "hello".to_string())]);
    }

    #[test]
    fn a_teams_folder_is_looked_for_on_the_disk_the_state_was_given() {
        let team = Team { name: TeamName::new("shop").unwrap(), folder: Some("/work/shop".into()), named: false };
        assert_eq!(team.live_folder(&edges::folders::FakeFolders(vec!["/work/shop".into()])), Some("/work/shop".into()));
        assert_eq!(team.live_folder(&edges::folders::FakeFolders(Vec::new())), None);
    }
}
