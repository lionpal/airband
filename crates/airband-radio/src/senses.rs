//! Why what he chose to listen or speak with cannot be used as things are: one typed reason, put in words for him
//! in one place. Listening (`ears.rs`) and the voice (`mouth.rs`) both carry it; `choices.rs` decides it.

/// Which of the two a reason is about: it names the place in Settings where he chooses again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sense {
    Listening,
    Voice,
}

impl Sense {
    fn section(self) -> &'static str {
        match self {
            Sense::Listening => "Listening",
            Sense::Voice => "Voice",
        }
    }
}

/// What he chose cannot be used, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cannot {
    /// A model this build cannot run on this device.
    ModelNotRun(Sense),
    /// A service this build cannot use for this.
    ServiceNotUsed(Sense),
    /// A model he chose that is not downloaded yet, by its name.
    NotDownloaded { sense: Sense, model: String },
    /// A voice model whose row names no voice of its own, and none was picked.
    NoVoiceNamed { model: String },
    /// A voice he picked that the model no longer has.
    VoiceGone { model: String, voice: String },
    /// A service that takes a key, with none kept on this phone for it: what the key is called.
    NoKey { sense: Sense, key_name: String },
}

impl Cannot {
    /// In words for him.
    pub(crate) fn told(&self) -> String {
        match self {
            Cannot::ModelNotRun(Sense::Listening) => "This version of airband cannot run the listening model you chose. Choose another in Settings, under Listening.".into(),
            Cannot::ModelNotRun(Sense::Voice) => "This version of airband cannot run the voice you chose. Choose another in Settings, under Voice.".into(),
            Cannot::ServiceNotUsed(Sense::Listening) => "This version of airband cannot listen with the service you chose. Choose another in Settings, under Listening.".into(),
            Cannot::ServiceNotUsed(Sense::Voice) => "This version of airband cannot speak with the service you chose. Choose another in Settings, under Voice.".into(),
            Cannot::NotDownloaded { sense, model } => format!("{model} is not on this phone yet. Download it in Settings, under {}.", sense.section()),
            Cannot::NoVoiceNamed { model } => format!("{model} names no voice of its own to speak in. Choose one in Settings, under Voice."),
            Cannot::VoiceGone { model, voice } => format!("{model} no longer has the voice {voice}. Choose another in Settings, under Voice."),
            Cannot::NoKey { sense, key_name } => format!("Enter your {key_name} in Settings, under {}.", sense.section()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_reason_names_the_place_he_chooses_again() {
        let all = [
            (Cannot::ModelNotRun(Sense::Listening), "Listening"),
            (Cannot::ModelNotRun(Sense::Voice), "Voice"),
            (Cannot::ServiceNotUsed(Sense::Listening), "Listening"),
            (Cannot::ServiceNotUsed(Sense::Voice), "Voice"),
            (Cannot::NotDownloaded { sense: Sense::Listening, model: "M".into() }, "Listening"),
            (Cannot::NoVoiceNamed { model: "M".into() }, "Voice"),
            (Cannot::VoiceGone { model: "M".into(), voice: "v".into() }, "Voice"),
        ];
        for (why, section) in all {
            assert!(why.told().ends_with(&format!("under {section}.")), "{why:?}");
        }
    }
}
