//! The coding tools, each in a folder of its own that holds everything of that tool's: its adapter (a `mod.rs`
//! that implements [`super::Harness`]), its account code, its tests and their fixtures. Nothing outside a tool's
//! folder and its row in `catalog/tools/` names it. A new tool is its row, a folder here, and a line in each of
//! the two lists below.
pub(crate) mod antigravity_cli;
pub(crate) mod claude_code;
pub(crate) mod codex;
pub(crate) mod opencode;

/// Every coding tool with an adapter, by the id its row has.
pub(crate) static HARNESSES: &[&dyn super::Harness] = &[&claude_code::ClaudeCode, &codex::Codex, &opencode::OpenCode, &antigravity_cli::AntigravityCli];
