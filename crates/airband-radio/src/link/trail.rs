//! The record of how each request went, kept for the connection screen.
use super::*;

/// The trail of one computer, kept in its file on this device: one for the computer, shared by every
/// connection to it, so only one copy of the file is ever written.
pub(crate) struct Trail {
    pub(super) lines: Mutex<std::collections::VecDeque<Line>>,
    pub(super) keeper: airband_io::kept::Keeper,
}

impl Trail {
    /// `file` is where it is kept between launches; nothing keeps it in memory only.
    pub(crate) fn open(file: Option<std::path::PathBuf>) -> Arc<Trail> {
        let (mut lines, keeper, damaged): (std::collections::VecDeque<Line>, _, _) = crate::kept::open(file, "The record of earlier requests");
        // the trail is for him to read on the connection screen: one that could not be read says so there
        if let Some(told) = damaged {
            lines.push_back(Line::Remark { at: now_ms(), what: told });
        }
        Arc::new(Trail { lines: Mutex::new(lines), keeper })
    }
}

/// One try at one request, one way: what the trail is made of. Nothing of what was said is in it, only the
/// kind of request, the way it went and how it ended. It never leaves the phone.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Step {
    pub(crate) at: u64,
    /// The method and the route, without anything after a `?`.
    pub(crate) what: String,
    pub(crate) way: Way,
    /// The status that came back, or what went wrong.
    pub(crate) outcome: Result<u16, LinkError>,
    pub(crate) ms: u64,
}

impl Step {
    /// One line of the diagnostics he can copy.
    pub(crate) fn line(&self) -> String {
        let way = match self.way {
            Way::Relay => "relay",
            Way::Near => "own network",
            Way::Kept => "kept at relay",
            Way::Paired => "direct",
        };
        let outcome = match &self.outcome {
            Ok(status) => format!("answered {status}"),
            Err(e) => e.told(),
        };
        format!("{} {} by {way}: {outcome} ({} ms)", clock(self.at), self.what, self.ms)
    }
}

/// One line of the trail: a step, or something worth a word that is not one.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum Line {
    Step(Step),
    Remark { at: u64, what: String },
}

impl Line {
    pub(super) fn told(&self) -> String {
        match self {
            Line::Step(step) => step.line(),
            Line::Remark { at, what } => format!("{} {what}", clock(*at)),
        }
    }
}

/// A time of day, for a line of the trail.
pub(super) fn clock(at: u64) -> String {
    let s = at / 1000;
    format!("{:02}:{:02}:{:02}Z", s / 3600 % 24, s / 60 % 60, s % 60)
}

/// How many lines the link remembers, for the diagnostics.
pub(super) const TRAIL: usize = 300;
