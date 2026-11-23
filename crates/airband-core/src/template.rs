//! A row's words with places in them (`{video}`, `{model}`): each place filled once, in one pass, so what goes
//! in is never read again as a place. A place with no value given is left as it is written.

/// `word` with each `{name}` that `values` has replaced by its value.
pub fn filled(word: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(word.len());
    let mut rest = word;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}').map(|close| (&after[..close], &after[close + 1..])) {
            Some((name, tail)) if let Some((_, value)) = values.iter().find(|(n, _)| *n == name) => {
                out.push_str(value);
                rest = tail;
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_put_in_once_and_never_read_again_as_a_place() {
        // A-N6: a video whose path had `{out}` in it was rewritten by the next replace
        assert_eq!(filled("-i {video} {out}/%03d.png", &[("video", "/v/{out}.mov"), ("out", "/f")]), "-i /v/{out}.mov /f/%03d.png");
        assert_eq!(filled("{unknown} and {model}", &[("model", "opus")]), "{unknown} and opus");
        assert_eq!(filled("no places {", &[("x", "y")]), "no places {");
    }
}
