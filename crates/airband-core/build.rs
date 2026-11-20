//! Puts the files under `catalog/` together into the one document that is compiled in (see `src/catalog.rs`).
use std::path::Path;

fn rows(dir: &Path) -> String {
    let mut files: Vec<_> = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect()).unwrap_or_default();
    files.sort();
    files.iter().map(|f| std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{}: {e}", f.display())).trim().to_string()).collect::<Vec<_>>().join(",\n")
}

fn main() {
    let catalog = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../catalog");
    println!("cargo:rerun-if-changed={}", catalog.display());
    for part in ["rev", "providers", "models"] {
        println!("cargo:rerun-if-changed={}", catalog.join(part).display());
    }
    let rev = std::fs::read_to_string(catalog.join("rev")).expect("catalog/rev");
    let document = format!("{{\"rev\":{},\"providers\":[\n{}\n],\"models\":[\n{}\n]}}\n", rev.trim(), rows(&catalog.join("providers")), rows(&catalog.join("models")));
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("catalog.json"), document).unwrap();
}
