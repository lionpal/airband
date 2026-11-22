//! The folder a command-line reader is run in, with its brief.
use super::*;

/// What a coding tool is told when it is used only to rewrite a message: one line, and no tools. Kept in a
/// folder of its own: as a plain file a tool can be pointed at, and in whatever other form a tool's row says
/// it looks for one (`brief` in the row). A general coding agent brings thousands of words of its
/// own instructions to every call; this is what keeps a summary small and quick.
pub(super) const BRIEF: &str = "You rewrite messages. Reply with the JSON asked for and nothing else.";

pub(super) fn brief_folder(by: &By) -> Result<std::path::PathBuf, String> {
    let folder = config::dir().join("brief");
    let mut files = vec![(folder.join("brief.md"), format!("{BRIEF}\n"))];
    // a tool that takes its brief as a file of its own kind, somewhere of its own inside the folder it runs
    // in: its row says where (`brief.path`) and what stands before the brief there (`brief.head`)
    if let Some(own) = by.with.get("brief") {
        let path = own["path"].as_str().map(std::path::Path::new).filter(|p| p.is_relative() && !p.components().any(|c| matches!(c, std::path::Component::ParentDir))).ok_or("the brief's place in this row is not inside the folder it runs in")?;
        files.push((folder.join(path), format!("{}{BRIEF}\n", own["head"].as_str().unwrap_or_default())));
    }
    if files.iter().any(|(path, text)| airband_io::files::read_text(path).ok().flatten().as_deref() != Some(text.as_str())) {
        for (path, text) in &files {
            airband_io::files::make_folder(path.parent().unwrap_or(&folder)).map_err(|e| e.to_string())?;
            airband_io::files::write_atomic(path, text).map_err(|e| e.to_string())?;
        }
    }
    Ok(folder)
}
