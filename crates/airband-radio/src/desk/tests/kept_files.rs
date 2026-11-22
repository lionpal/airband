//! The phone's own files: one that could not be read is told once, and the team colours file holds only teams
//! that are still there.
use super::*;

struct Noted(Mutex<Vec<String>>);
impl DeskHost for Noted {
    fn secret(&self, _: String) -> Option<String> {
        None
    }
    fn keep_secret(&self, _: String, _: Option<String>) {}
    fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
    fn record_start(&self, _: u32) -> bool {
        false
    }
    fn record_read(&self) -> Vec<u8> {
        Vec::new()
    }
    fn record_stop(&self) {}
    fn changed(&self) {}
    fn status(&self, _: RadioStatus) {}
    fn note(&self, text: String) {
        self.0.lock().unwrap().push(text);
    }
    fn hearing(&self, _: String) {}
    fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
    fn watching(&self) -> bool {
        true
    }
    fn device(&self) -> HostDevice {
        HostDevice::default()
    }
    fn run_model(&self, _: String, _: String, _: ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
        None
    }
}

#[test]
fn a_file_of_his_that_could_not_be_read_is_told_once_as_the_app_opens() {
    let dir = temp_dir("damaged");
    for file in ["read.json", "colours.json", "choices.json"] {
        std::fs::write(dir.join(file), b"{ half written").unwrap();
    }
    let host = Arc::new(Noted(Mutex::new(Vec::new())));
    let _desk = Desk::open(dir.display().to_string(), host.clone());
    let notes = host.0.lock().unwrap().clone();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert!(notes.iter().all(|n| n.contains("could not be read") && n.contains("old copy is kept")), "{notes:?}");
    assert!(notes.iter().any(|n| n.starts_with("Team colours")) && notes.iter().any(|n| n.starts_with("What you chose in Settings")), "{notes:?}");
}

#[test]
fn files_that_read_tell_him_nothing() {
    let dir = temp_dir("fine");
    let host = Arc::new(Noted(Mutex::new(Vec::new())));
    let _desk = Desk::open(dir.display().to_string(), host.clone());
    let notes = host.0.lock().unwrap().clone();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn a_colour_kept_for_a_team_no_computer_has_is_let_go_and_the_rest_stay() {
    let dir = temp_dir("colours-prune");
    let mut colours = super::super::colours::Colours::open(dir.to_str().unwrap(), 6);
    let _ = colours.see(["mini/shop".to_string(), "mini/garden".to_string(), "laptop/shop".to_string()]);
    let still: std::collections::BTreeSet<String> = ["mini/shop".to_string(), "laptop/shop".to_string()].into();
    let lost = colours.keep_only(&still);
    assert!(lost.is_none());
    let again = super::super::colours::Colours::open(dir.to_str().unwrap(), 6);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(again.of("mini/garden"), None, "a gone team keeps its colour on disk");
    assert_eq!((again.of("mini/shop"), again.of("laptop/shop")), (colours.of("mini/shop"), colours.of("laptop/shop")));
}
