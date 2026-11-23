//! Programs a row names for the tower to run: a terminal a message is typed into, and one that pulls frames out
//! of a video.
use super::*;

/// A terminal a message can be typed into, as its row has it. In `type` and `enter`, `{pane}` is the pane and
/// `{line}` the words; each is one argument, never read by a shell.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Terminal {
    pub id: String,
    pub program: String,
    #[serde(rename = "type")]
    pub type_line: Vec<String>,
    pub enter: Vec<String>,
}

impl Terminal {
    /// The two runs of the program that type `line` into `pane` and press Enter.
    pub fn runs(&self, pane: &str, line: &str) -> [Vec<String>; 2] {
        let fill = |words: &[String]| words.iter().map(|w| crate::template::word(w, &[("pane", pane), ("line", line)])).collect();
        [fill(&self.type_line), fill(&self.enter)]
    }
}

/// A program that pulls a frame a second out of a video, so an agent has pictures it can look at.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Frames {
    pub id: String,
    /// What it is called, for telling an agent which program to use.
    pub name: String,
    pub program: String,
    /// The words it is run with: `{video}` the video, `{frames}` how many at most, `{out}` the folder they go in.
    pub args: Vec<String>,
}

impl Frames {
    /// The words to run it with, for this video, this many frames, into this folder.
    pub fn run(&self, video: &str, frames: usize, out: &str) -> Vec<String> {
        let frames = frames.to_string();
        self.args.iter().map(|w| crate::template::filled(w, &[("video", video), ("frames", &frames), ("out", out)])).collect()
    }
}

/// A package manager airband is put on a computer with (`catalog/installers/`): what is pasted in a terminal to
/// install it, and how to ask it for the folder its copy stays reachable at across its updates. Each is one
/// argument, never read by a shell.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Installer {
    pub id: String,
    /// What it is called, to tell him which copy is used.
    pub name: String,
    /// The words that install airband with it.
    pub install: Vec<String>,
    /// A run of it that prints, alone on its output, the folder that stays airband's own across its updates.
    pub lasting: Vec<String>,
    /// The folder under that one the binary is in.
    pub bin: String,
}

impl Installer {
    /// What is pasted in a terminal to install airband with it.
    pub fn install_line(&self) -> String {
        self.install.join(" ")
    }
}

/// Every package manager this build can install airband with, as its rows list them. None is picked here:
/// a caller takes the one a fact points at (the one whose copy is running), or offers each.
pub fn installers() -> Vec<Installer> {
    current().installers.clone()
}

/// Who makes airband, to write to from the app: what to call him and the number a text message goes to.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Contact {
    pub id: String,
    pub name: String,
    /// The number a text message to him goes to, in international form (`+` and digits).
    pub sms: String,
}

/// Who makes airband. The catalogue has exactly one (`form::check`).
pub fn contact() -> Option<Contact> {
    match current().contact.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

/// Every program this build can pull frames out of a video with. A caller tries each until one runs.
pub fn frames_programs() -> Vec<Frames> {
    current().frames.clone()
}

/// The terminal a row by this id describes.
pub fn terminal(id: &str) -> Option<Terminal> {
    current().terminals.iter().find(|t| t.id == id).cloned()
}

#[cfg(test)]
mod frames_tests {
    use super::*;

    #[test]
    fn a_path_holding_a_placeholder_is_put_in_as_it_is() {
        let f = Frames { id: "f".into(), name: "f".into(), program: "p".into(), args: vec!["-i".into(), "{video}".into(), "-frames".into(), "{frames}".into(), "{out}/%04d.jpg".into(), "{other}".into()] };
        assert_eq!(f.run("/v/{out}.mov", 3, "/o"), ["-i", "/v/{out}.mov", "-frames", "3", "/o/%04d.jpg", "{other}"]);
    }
}
