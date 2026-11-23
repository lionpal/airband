//! A row's words with places in them (`{video}`, `{model}`): each place filled once, in one pass, so what goes
//! in is never read again as a place. A place with no value given is left as it is written. The one reader of
//! places in the core is [`pieces`]: [`filled`] fills places anywhere in a word, [`word`] only a word that is
//! wholly one place (for a program's arguments, where a value must stay one whole word), and the core's other
//! fillers (a key check's address, an endpoint's path, a relay route) are made of these.

/// One piece of a text with places in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece<'a> {
    /// Words as written.
    Text(&'a str),
    /// A place: the name between its braces.
    Place(&'a str),
    /// A `{` with no `}` after it, and everything after it.
    Unclosed(&'a str),
}

/// `text` as its pieces, in order.
pub fn pieces(text: &str) -> impl Iterator<Item = Piece<'_>> {
    let mut rest = Some(text);
    let mut place = None;
    std::iter::from_fn(move || {
        if let Some(name) = place.take() {
            return Some(Piece::Place(name));
        }
        let now = rest.take()?;
        let Some(open) = now.find('{') else { return (!now.is_empty()).then_some(Piece::Text(now)) };
        let (before, after) = (&now[..open], &now[open + 1..]);
        match after.find('}') {
            // a `{` that another opens after before any closes is words: the place, if any, starts later
            Some(close) if after[..close].contains('{') => {
                rest = Some(after);
                Some(Piece::Text(&now[..open + 1]))
            }
            Some(close) => {
                rest = Some(&after[close + 1..]);
                if before.is_empty() {
                    return Some(Piece::Place(&after[..close]));
                }
                place = Some(&after[..close]);
                Some(Piece::Text(before))
            }
            None if before.is_empty() => Some(Piece::Unclosed(now)),
            None => {
                rest = Some(&now[open..]);
                Some(Piece::Text(before))
            }
        }
    })
}

/// The value `word` names when it is wholly one place (`{name}`) that `values` has; else the word as written.
/// What goes in is always one whole word: what he said can never become an option or two arguments.
pub fn word(word: &str, values: &[(&str, &str)]) -> String {
    let mut all = pieces(word);
    match (all.next(), all.next()) {
        (Some(Piece::Place(_)), None) => filled(word, values),
        _ => word.to_string(),
    }
}

/// Whether `word` has the place `{name}` in it, as its pieces read: asked of a row's word before anything is
/// filled in, so a value never counts as a place.
pub fn has_place(word: &str, name: &str) -> bool {
    pieces(word).any(|piece| piece == Piece::Place(name))
}

/// `word` with each `{name}` that `values` has replaced by its value.
pub fn filled(word: &str, values: &[(&str, &str)]) -> String {
    pieces(word)
        .map(|piece| match piece {
            Piece::Place(name) => values.iter().find(|(n, _)| *n == name).map_or_else(|| format!("{{{name}}}"), |(_, v)| v.to_string()),
            Piece::Text(text) | Piece::Unclosed(text) => text.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_put_in_once_and_never_read_again_as_a_place() {
        // a video whose path had `{out}` in it was rewritten by the next replace
        assert_eq!(filled("-i {video} {out}/%03d.png", &[("video", "/v/{out}.mov"), ("out", "/f")]), "-i /v/{out}.mov /f/%03d.png");
        assert_eq!(filled("{unknown} and {model}", &[("model", "opus")]), "{unknown} and opus");
        assert_eq!(filled("no places {", &[("x", "y")]), "no places {");
        assert_eq!(filled("{a}{b}x{", &[("a", "1"), ("b", "{a}")]), "1{a}x{");
        assert_eq!(filled("a { b {c} d", &[("c", "C")]), "a { b C d");
        assert_eq!(pieces("{url}/m/{model}").collect::<Vec<_>>(), [Piece::Place("url"), Piece::Text("/m/"), Piece::Place("model")]);
        assert_eq!(pieces("x{y").collect::<Vec<_>>(), [Piece::Text("x"), Piece::Unclosed("{y")]);
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
