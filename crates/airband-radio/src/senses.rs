//! Why what he chose to listen or speak with cannot be used as things are, or why nothing is chosen: one typed
//! reason, put in words for him in this one place. Listening (`ears.rs`) and the voice (`mouth.rs`, `voice.rs`,
//! `voices.rs`) all carry it; `choices.rs` decides it.

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
    /// Nothing is chosen.
    NothingChosen(Sense),
    /// A service this build does not know at all.
    UnknownService(Sense),
    /// A service this build knows, set up in a way it cannot use, in the words of what said so (a rate that
    /// is not one, a field the row does not give).
    BadSetting { why: String },
    /// A model this build cannot run on this device.
    ModelNotRun(Sense),
    /// A service this build cannot use for this.
    ServiceNotUsed(Sense),
    /// A model he chose that is not downloaded yet, by its name. Its card in Settings offers the download.
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
            Cannot::NothingChosen(Sense::Listening) => "Nothing is chosen to listen with on this device. Choose it in Settings, under Listening.".into(),
            Cannot::NothingChosen(Sense::Voice) => "No voice is chosen on this device. Choose one in Settings, under Voice.".into(),
            Cannot::UnknownService(Sense::Listening) => "This version of airband does not know that listening service.".into(),
            Cannot::UnknownService(Sense::Voice) => "This version of airband does not know that voice service.".into(),
            Cannot::BadSetting { why } => why.clone(),
            Cannot::ModelNotRun(Sense::Listening) => "This version of airband cannot run the listening model you chose. Choose another in Settings, under Listening.".into(),
            Cannot::ModelNotRun(Sense::Voice) => "This version of airband cannot run the voice you chose. Choose another in Settings, under Voice.".into(),
            Cannot::ServiceNotUsed(Sense::Listening) => "This version of airband cannot listen with the service you chose. Choose another in Settings, under Listening.".into(),
            Cannot::ServiceNotUsed(Sense::Voice) => "This version of airband cannot speak with the service you chose. Choose another in Settings, under Voice.".into(),
            Cannot::NotDownloaded { model, .. } => format!("{model} is not on this phone yet."),
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
            (Cannot::NoVoiceNamed { model: "M".into() }, "Voice"),
            (Cannot::VoiceGone { model: "M".into(), voice: "v".into() }, "Voice"),
            (Cannot::NothingChosen(Sense::Listening), "Listening"),
            (Cannot::NothingChosen(Sense::Voice), "Voice"),
        ];
        for (why, section) in all {
            assert!(why.told().ends_with(&format!("under {section}.")), "{why:?}");
        }
        // one not downloaded yet is downloaded from its card, which is where he reads this: it sends him nowhere
        assert_eq!(Cannot::NotDownloaded { sense: Sense::Listening, model: "M".into() }.told(), "M is not on this phone yet.");
    }
}
