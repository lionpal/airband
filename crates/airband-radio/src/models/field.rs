//! The names of the fields of a row's `with` that this build reads, each written once.
/// A model row: the one to start with for its job.
pub(crate) const USUAL: &str = "usual";
/// A model row: what the device does once before first use, in words for him.
pub(crate) const PREPARE: &str = "prepare";
/// A model row: the lowest system it runs on, for each kind of device.
pub(crate) const MIN_OS: &str = "min_os";
/// A voice: the rate of its sound.
pub(crate) const RATE: &str = "rate";
/// A voice service's setting: which of its voices speaks.
pub(crate) const VOICE: &str = "voice";
/// A service row: further headers each request carries.
pub(crate) const HEADERS: &str = "headers";
/// A service's setting: where it is reached.
pub(crate) const URL: &str = "url";
/// A service's setting: which of its models does the work.
pub(crate) const MODEL: &str = "model";
/// A listening service's setting, or a voice row's: the language he speaks.
pub(crate) const LANGUAGE: &str = "language";
/// A service row: how his key is presented.
pub(crate) const AUTH: &str = "auth";
/// A voice row: how the words are to be said, sent beside them and never spoken.
pub(crate) const STYLE: &str = "style";
/// A live listening row: where its stream is reached.
pub(crate) const STREAM: &str = "stream";
/// A live listening row: where a pass for the stream is asked for.
pub(crate) const PASS: &str = "pass";
/// A model row for the device's own voice: how its weights are packed, its language, and what runs it.
#[cfg(feature = "phonon")]
pub(crate) const QUANT: &str = "quant";
#[cfg(feature = "phonon")]
pub(crate) const LANG: &str = "lang";
#[cfg(feature = "phonon")]
pub(crate) const DEVICE: &str = "device";
