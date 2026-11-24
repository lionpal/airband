//! How long each download of a model and each getting it ready took on this device, kept for the connection
//! record: what words about how long these take can be based on. Nothing he said is in it.
use super::*;

/// How many are kept, newest last.
const KEPT: usize = 12;

/// What was done with a model, and how long it took.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "did")]
pub(crate) enum Did {
    /// Its files came, whole and checked: this many bytes in all.
    Downloaded { bytes: u64, ms: u64 },
    /// The download ended without them, with this many bytes here, and why.
    NotDownloaded { bytes: u64, ms: u64, why: String },
    /// The device got it ready.
    Readied { ms: u64 },
    /// The device could not get it ready, and why.
    NotReadied { ms: u64, why: String },
}

/// One of them, with when and which model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Timing {
    /// Milliseconds since 1970, when it ended.
    pub(crate) at: u64,
    /// The model's kind, and its version.
    pub(crate) model: String,
    pub(crate) version: String,
    #[serde(flatten)]
    pub(crate) did: Did,
}

/// The kept list, in `model-timings.json` in the app's folder.
pub(crate) struct Timings {
    keeper: airband_io::kept::Keeper,
    kept: Mutex<Vec<Timing>>,
}

impl Timings {
    pub(crate) fn open(dir: &str) -> Timings {
        // what could not be read is started again: it is a record, and nothing he chose is in it
        let (kept, keeper, _) = crate::kept::open::<Vec<Timing>>(crate::kept::file(dir, "model-timings.json"), "How long downloads took");
        Timings { keeper, kept: Mutex::new(kept) }
    }

    pub(crate) fn note(&self, m: &Model, did: Did) {
        let mut kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
        kept.push(Timing { at: airband_core::clock::now_ms(), model: m.kind.clone(), version: m.version.clone(), did });
        let extra = kept.len().saturating_sub(KEPT);
        kept.drain(..extra);
        if let Err(why) = crate::kept::said(self.keeper.save(&*kept)) {
            eprintln!("airband: {why}");
        }
    }

    pub(crate) fn all(&self) -> Vec<Timing> {
        self.kept.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// As lines for the connection record.
    pub(crate) fn lines(&self) -> Vec<String> {
        self.all().iter().map(Timing::line).collect()
    }
}

impl Timing {
    fn line(&self) -> String {
        let (when, what) = (airband_core::clock::rfc3339(self.at), format!("{} version {}", self.model, self.version));
        match &self.did {
            Did::Downloaded { bytes, ms } => format!("{when} {what}: downloaded {} MB in {}", crate::models::megabytes(*bytes), took(*ms)),
            Did::NotDownloaded { bytes, ms, why } => format!("{when} {what}: download ended after {} with {} MB here: {why}", took(*ms), crate::models::megabytes(*bytes)),
            Did::Readied { ms } => format!("{when} {what}: got ready in {}", took(*ms)),
            Did::NotReadied { ms, why } => format!("{when} {what}: could not be got ready, after {}: {why}", took(*ms)),
        }
    }
}

/// A length of time as the record says it: seconds, to the tenth.
fn took(ms: u64) -> String {
    format!("{:.1} s", ms as f64 / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_download_and_getting_ready_is_kept_with_how_long_it_took_and_only_so_many() {
        let dir = std::env::temp_dir().join(format!("airband-timings-{}", random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let at = dir.display().to_string();
        let m = crate::models::models(&HostDevice { engines: airband_core::catalog::current().models.iter().map(|m| m.engine.clone()).collect(), os: "ios".into(), os_version: u32::MAX, team_colours: 0, os_build: String::new() }).next().unwrap();
        let t = Timings::open(&at);
        t.note(&m, Did::Downloaded { bytes: 66_000_000, ms: 41_260 });
        t.note(&m, Did::Readied { ms: 192_000 });
        let lines = Timings::open(&at).lines();
        assert!(lines[0].ends_with(&format!("{} version {}: downloaded 66 MB in 41.3 s", m.kind, m.version)), "{lines:?}");
        assert!(lines[1].ends_with("got ready in 192.0 s"), "{lines:?}");
        for _ in 0..KEPT + 3 {
            t.note(&m, Did::NotReadied { ms: 5, why: "it could not be loaded".into() });
        }
        assert_eq!(Timings::open(&at).all().len(), KEPT);
        let _ = std::fs::remove_dir_all(dir);
    }
}
