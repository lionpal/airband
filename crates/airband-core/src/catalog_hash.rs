// What the rows of a catalogue hash to. Shared, as it is written, by `build.rs` (which writes the hash into
// the document) and by `catalog.rs`: the rows in a form that does not depend on spacing or on the order of
// a row's fields, then SHA-256 in hex.
fn content_hash(doc: &serde_json::Value) -> String {
    use sha2::Digest;
    fn canon(v: &serde_json::Value, out: &mut String) {
        match v {
            serde_json::Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                out.push('{');
                for k in keys {
                    out.push_str(&serde_json::Value::String(k.clone()).to_string());
                    out.push(':');
                    canon(&map[k], out);
                    out.push(',');
                }
                out.push('}');
            }
            serde_json::Value::Array(items) => {
                out.push('[');
                for item in items {
                    canon(item, out);
                    out.push(',');
                }
                out.push(']');
            }
            other => out.push_str(&other.to_string()),
        }
    }
    let mut rows = String::new();
    // the kinds of link, the shapes of credentials and the terminals are not in it: a build only ever uses its own
    for part in ["providers", "models", "tools", "servers", "phrases"] {
        canon(&doc[part], &mut rows);
    }
    sha2::Sha256::digest(rows.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}
