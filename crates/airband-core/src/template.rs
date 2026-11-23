//! A row's words with places in them (`{video}`, `{model}`): each place filled once, in one pass, so what goes
//! in is never read again as a place. A place with no value given is left as it is written. The one filler of
//! the core: [`filled`] fills places anywhere in a word, [`word`] only a word that is wholly one place, for a
//! program's arguments, where a value must stay one whole word.

/// The value `word` names when it is wholly one place (`{name}`) that `values` has; else the word as written.
/// What goes in is always one whole word: what he said can never become an option or two arguments.
pub fn word(word: &str, values: &[(&str, &str)]) -> String {
    let name = word.strip_prefix('{').and_then(|w| w.strip_suffix('}'));
    values.iter().find(|(n, _)| Some(*n) == name).map_or_else(|| word.to_string(), |(_, v)| v.to_string())
}

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
    #[test]
    fn a_whole_word_place_takes_its_value_as_one_word_and_nothing_else_is_filled() {
        let v = [("line", "rm -rf {pane}"), ("pane", "%3")];
        assert_eq!(word("{line}", &v), "rm -rf {pane}");
        assert_eq!(word("{pane}", &v), "%3");
        assert_eq!(word("-t{pane}", &v), "-t{pane}");
        assert_eq!(word("{other}", &v), "{other}");
        assert_eq!(word("{", &v), "{");
    }
}
