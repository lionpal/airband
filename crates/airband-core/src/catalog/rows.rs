//! Reading the catalogue's rows from their files, and the document they make.
use super::*;

#[derive(Deserialize)]
pub(super) struct Document {
    rev: u64,
    #[serde(default)]
    hash: String,
    #[serde(default)]
    providers: Vec<Value>,
    #[serde(default)]
    models: Vec<Value>,
    #[serde(default)]
    tools: Vec<ToolRow>,
    #[serde(default)]
    servers: Vec<Value>,
    #[serde(default)]
    links: Vec<Value>,
    #[serde(default)]
    secrets: Vec<Value>,
    #[serde(default)]
    terminals: Vec<Value>,
    #[serde(default)]
    phrases: Vec<String>,
    #[serde(default)]
    retired: Vec<String>,
}

pub(super) fn yes() -> bool {
    true
}

/// A provider's row, but for its offers, which are read one at a time.
#[derive(Deserialize)]
pub(super) struct ProviderHead {
    id: String,
    name: String,
    /// Present when it takes a key: what the key is called, and where keys are made.
    #[serde(default)]
    key: Option<KeyRow>,
    #[serde(default = "yes")]
    offered: bool,
    #[serde(default)]
    offers: Vec<Value>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ToolRow {
    id: String,
    name: String,
    #[serde(default)]
    maker: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    first: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    launchers: Vec<crate::tools::Launcher>,
    #[serde(default)]
    with: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
pub(super) struct KeyRow {
    name: String,
    #[serde(default)]
    at: String,
    /// How the key is tested when he gives it (`providers::Check`).
    check: crate::providers::Check,
}

/// What the rows of a document hash to (`build.rs` writes it into the document; see `catalog_hash.rs`).
pub fn hash_of(document: &str) -> Result<String, String> {
    serde_json::from_str::<Value>(document).map(|doc| content_hash(&doc)).map_err(|e| e.to_string())
}
include!("../catalog_hash.rs");

pub(super) fn read(text: &str) -> Result<Catalog, String> {
    let doc: Document = serde_json::from_str(text).map_err(|e| format!("the catalogue does not read: {e}"))?;
    let mut left_out = Vec::new();
    let mut providers = Vec::new();
    for row in doc.providers {
        let named = row["id"].as_str().unwrap_or("a provider with no id").to_string();
        let head: ProviderHead = match serde_json::from_value(row) {
            Ok(head) => head,
            Err(e) => {
                left_out.push(format!("{named}: {e}"));
                continue;
            }
        };
        let mut offers = Vec::new();
        for o in head.offers {
            match serde_json::from_value::<Offer>(o) {
                Ok(o) => offers.push(Arc::new(o)),
                Err(e) => left_out.push(format!("{}: an offer ({e})", head.id)),
            }
        }
        let (key, key_name, keys_at, check) = match head.key {
            Some(k) => (true, k.name, k.at, Some(k.check)),
            None => (false, String::new(), String::new(), None),
        };
        providers.push(Arc::new(Provider { id: head.id, name: head.name, key, key_name, keys_at, check, offered: head.offered, offers }));
    }
    let mut models = Vec::new();
    for row in doc.models {
        let named = row["kind"].as_str().unwrap_or("a model with no name").to_string();
        match serde_json::from_value::<PhoneModel>(row) {
            Ok(m) => models.push(Arc::new(m)),
            Err(e) => left_out.push(format!("{named}: {e}")),
        }
    }
    let servers = doc.servers.into_iter().filter_map(|row| serde_json::from_value::<Server>(row).ok()).collect();
    let links = doc.links.into_iter().filter_map(|row| serde_json::from_value::<crate::links::LinkKind>(row).ok()).collect();
    let terminals = doc.terminals.into_iter().filter_map(|row| serde_json::from_value::<Terminal>(row).ok()).collect();
    let secrets = doc.secrets.into_iter().filter_map(|row| serde_json::from_value::<crate::redact::Shape>(row).ok()).collect();
    let tools = doc
        .tools
        .into_iter()
        .map(|t| {
            // a launcher in a tool's row starts that tool
            let launchers = t.launchers.into_iter().map(|l| crate::tools::Launcher { harness: t.id.clone(), ..l }).collect();
            Tool { id: t.id, name: t.name, maker: t.maker, first: t.first, launchers, with: t.with }
        })
        .collect();
    Ok(Catalog { rev: doc.rev, hash: doc.hash, providers, tools, models, servers, links, secrets, terminals, phrases: doc.phrases, retired: doc.retired, left_out })
}

/// A terminal a message can be typed into, as its row has it. In `type` and `enter`, `{pane}` is the pane and
/// `{line}` the words; each is one argument, never read by a shell.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Terminal {
    pub id: String,
    pub program: String,
    #[serde(rename = "type")]
    pub type_line: Vec<String>,
    pub enter: Vec<String>,
}

impl Terminal {
    /// The two runs of the program that type `line` into `pane` and press Enter.
    pub fn runs(&self, pane: &str, line: &str) -> [Vec<String>; 2] {
        let fill = |words: &[String]| words.iter().map(|w| if w == "{pane}" { pane.to_string() } else if w == "{line}" { line.to_string() } else { w.clone() }).collect();
        [fill(&self.type_line), fill(&self.enter)]
    }
}

/// The terminal a row by this id describes.
pub fn terminal(id: &str) -> Option<Terminal> {
    current().terminals.iter().find(|t| t.id == id).cloned()
}

// ── a copy kept on this device, and the published one ──
