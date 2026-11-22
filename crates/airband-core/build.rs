//! Puts the files under `catalog/` together into the one document that is compiled in (see `src/catalog.rs`).
use std::path::Path;

include!("src/catalog_hash.rs");

fn rows(dir: &Path) -> String {
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())).flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    files.iter().map(|f| std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{}: {e}", f.display())).trim().to_string()).collect::<Vec<_>>().join(",\n")
}

fn main() {
    let catalog = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../catalog");
    println!("cargo:rerun-if-changed={}", catalog.display());
    for part in ["rev", "providers", "models", "tools", "servers", "links", "secrets", "terminals", "frames", "installers", "phrases.json"] {
        println!("cargo:rerun-if-changed={}", catalog.join(part).display());
    }
    // `catalog/rev`: the number, and under it what the rows hashed to when the number was given
    let stamped = std::fs::read_to_string(catalog.join("rev")).expect("catalog/rev");
    let rev = stamped.lines().next().unwrap_or_default().trim();
    let phrases = std::fs::read_to_string(catalog.join("phrases.json")).expect("catalog/phrases.json");
    let document = format!(
        "{{\"rev\":{rev},\"providers\":[\n{}\n],\"models\":[\n{}\n],\"tools\":[\n{}\n],\"servers\":[\n{}\n],\"links\":[\n{}\n],\"secrets\":[\n{}\n],\"terminals\":[\n{}\n],\"frames\":[\n{}\n],\"installers\":[\n{}\n],\"phrases\":{}}}\n",
        rows(&catalog.join("providers")),
        rows(&catalog.join("models")),
        rows(&catalog.join("tools")),
        rows(&catalog.join("servers")),
        rows(&catalog.join("links")),
        rows(&catalog.join("secrets")),
        rows(&catalog.join("terminals")),
        rows(&catalog.join("frames")),
        rows(&catalog.join("installers")),
        phrases.trim()
    );
    // what was just written must read, and must have something in it: a build made where the rows are
    // missing or broken stops here, not on someone's computer
    let mut doc: serde_json::Value = serde_json::from_str(&document).unwrap_or_else(|e| panic!("the files under catalog/ do not make a document: {e}"));
    for part in ["providers", "models", "tools", "servers", "links", "secrets", "terminals", "frames", "installers", "phrases"] {
        assert!(doc[part].as_array().is_some_and(|rows| !rows.is_empty()), "catalog/{part} is empty");
    }
    doc["hash"] = content_hash(&doc).into();
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("catalog.json"), doc.to_string()).unwrap();
}
