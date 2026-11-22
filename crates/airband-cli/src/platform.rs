//! The operating system's own programs airband runs, each by its full path so a program of the same name earlier
//! on his PATH is never run in its place. These are the system's, not a coding tool's or a service's: they are
//! not catalogue rows.

/// The service manager that keeps the tower running (macOS).
pub(crate) const LAUNCHCTL: &str = "/bin/launchctl";
/// Process status: when a process started, to tell a process from a later one given the same number.
pub(crate) const PS: &str = "/bin/ps";
/// Unpacks a downloaded model.
pub(crate) const TAR: &str = "/usr/bin/tar";
/// Runs one command line.
pub(crate) const SH: &str = "/bin/sh";
/// The computer's name as he set it (macOS).
pub(crate) const SCUTIL: &str = "/usr/sbin/scutil";
/// The computer's short host name.
pub(crate) const HOSTNAME: &str = "/bin/hostname";
/// The login keychain (macOS).
pub(crate) const SECURITY: &str = "/usr/bin/security";
