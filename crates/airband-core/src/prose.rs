//! What an agent wrote, read as the markdown it is: headings, lists inside lists, tables, quotes, code. A host
//! draws the blocks; it never reads the marks itself. The same blocks give the words for reading a whole
//! message aloud, so no mark is ever spoken.
//!
//! The reading is CommonMark with the tables, strikethrough and task lists agents use. Anything written as
//! HTML is shown as the text it is and never acted on. Line breaks inside a paragraph are kept as written.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Where each picture in a piece of Markdown points (`![alt](here)`), in order, as the parser finds them.
pub fn images(text: &str) -> Vec<String> {
    Parser::new_ext(text, Options::empty()).filter_map(|event| match event {
        Event::Start(Tag::Image { dest_url, .. }) => Some(dest_url.to_string()),
        _ => None,
    }).collect()
}

/// A run of words in one style.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ProseSpan {
    pub text: String,
    pub strong: bool,
    pub emphasis: bool,
    pub strike: bool,
    pub code: bool,
    /// Where it leads, when it is a link.
    pub link: Option<String>,
}

/// One thing in a list, which may hold anything, another list included.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ProseItem {
    /// A task: done or not. Nothing for an ordinary item.
    pub task: Option<bool>,
    pub blocks: Vec<ProseBlock>,
}

/// Which side a table's column keeps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ProseSide {
    Left,
    Middle,
    Right,
}

/// One cell of a table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ProseCell {
    pub spans: Vec<ProseSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ProseBlock {
    Paragraph { spans: Vec<ProseSpan> },
    /// `level` is 1 for the largest.
    Heading { level: u8, spans: Vec<ProseSpan> },
    /// `start` is the number of the first item of a numbered list; nothing for bullets.
    List { start: Option<u64>, items: Vec<ProseItem> },
    /// As it was written, every line and space. `language` is what the fence named, or empty.
    Code { language: String, text: String },
    Quote { blocks: Vec<ProseBlock> },
    /// `sides` has one entry for each column.
    Table { sides: Vec<ProseSide>, head: Vec<ProseCell>, rows: Vec<Vec<ProseCell>> },
    Rule,
}

/// What an agent wrote, as blocks to draw.
pub fn blocks(text: &str) -> Vec<ProseBlock> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut reader = Reader { events: Parser::new_ext(text, options).collect(), at: 0 };
    reader.blocks()
}

struct Reader<'a> {
    events: Vec<Event<'a>>,
    at: usize,
}

/// The styles that are open where the reader stands.
#[derive(Default)]
struct Open {
    strong: u32,
    emphasis: u32,
    strike: u32,
    links: Vec<String>,
}

impl Reader<'_> {
    fn next(&mut self) -> Option<Event<'_>> {
        let event = self.events.get(self.at).cloned();
        self.at += 1;
        event
    }

    /// Blocks up to the end of whatever holds them. The end itself is taken.
    fn blocks(&mut self) -> Vec<ProseBlock> {
        let mut out = Vec::new();
        while let Some(event) = self.events.get(self.at).cloned() {
            match event {
                Event::End(_) => {
                    self.at += 1;
                    break;
                }
                Event::Start(Tag::Paragraph) => {
                    self.at += 1;
                    let spans = self.spans();
                    self.at += 1; // its end
                    out.push(ProseBlock::Paragraph { spans });
                }
                Event::Start(Tag::Heading { level, .. }) => {
                    self.at += 1;
                    let spans = self.spans();
                    self.at += 1;
                    out.push(ProseBlock::Heading { level: level as u8, spans });
                }
                Event::Start(Tag::BlockQuote(_)) => {
                    self.at += 1;
                    out.push(ProseBlock::Quote { blocks: self.blocks() });
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    self.at += 1;
                    let language = match kind {
                        // the fence may say more than the language ("rust,ignore"): the first word is the language
                        CodeBlockKind::Fenced(named) => named.split([' ', ',']).next().unwrap_or_default().to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    let mut text = String::new();
                    while let Some(event) = self.next() {
                        match event {
                            Event::End(_) => break,
                            Event::Text(part) => text.push_str(&part),
                            _ => {}
                        }
                    }
                    out.push(ProseBlock::Code { language, text: text.trim_end_matches('\n').to_string() });
                }
                Event::Start(Tag::List(start)) => {
                    self.at += 1;
                    let mut items = Vec::new();
                    while let Some(Event::Start(Tag::Item)) = self.events.get(self.at) {
                        self.at += 1;
                        let task = match self.events.get(self.at) {
                            Some(Event::TaskListMarker(done)) => {
                                let done = *done;
                                self.at += 1;
                                Some(done)
                            }
                            _ => None,
                        };
                        items.push(ProseItem { task, blocks: self.blocks() });
                    }
                    self.at += 1; // the list's end
                    out.push(ProseBlock::List { start, items });
                }
                Event::Start(Tag::Table(sides)) => {
                    self.at += 1;
                    let sides = sides.iter().map(|side| match side {
                        Alignment::Right => ProseSide::Right,
                        Alignment::Center => ProseSide::Middle,
                        Alignment::Left | Alignment::None => ProseSide::Left,
                    });
                    let (mut head, mut rows) = (Vec::new(), Vec::new());
                    while let Some(event) = self.next() {
                        match event {
                            Event::Start(Tag::TableHead) => head = self.cells(),
                            Event::Start(Tag::TableRow) => rows.push(self.cells()),
                            _ => break, // the table's end
                        }
                    }
                    out.push(ProseBlock::Table { sides: sides.collect(), head, rows });
                }
                Event::Rule => {
                    self.at += 1;
                    out.push(ProseBlock::Rule);
                }
                // anything else here is words with nothing around them: the text of a list item written on one
                // line, or HTML, which is shown as the text it is
                _ => {
                    let spans = self.spans();
                    if spans.is_empty() {
                        if matches!(self.events.get(self.at), Some(Event::Start(_))) {
                            // something this reader has no drawing for (a block of HTML, a footnote's body):
                            // what is inside it is shown as it stands
                            self.at += 1;
                            let inside = self.blocks();
                            out.extend(inside);
                        } else if !matches!(self.events.get(self.at), Some(Event::End(_)) | None) {
                            self.at += 1;
                        }
                    } else {
                        out.push(ProseBlock::Paragraph { spans });
                    }
                }
            }
        }
        out
    }

    /// The cells of one row of a table, up to and with the row's end.
    fn cells(&mut self) -> Vec<ProseCell> {
        let mut cells = Vec::new();
        while let Some(Event::Start(Tag::TableCell)) = self.events.get(self.at) {
            self.at += 1;
            cells.push(ProseCell { spans: self.spans() });
            self.at += 1; // the cell's end
        }
        self.at += 1;
        cells
    }

    /// Words in their styles, up to the first thing that is not words. That thing is left where it is.
    fn spans(&mut self) -> Vec<ProseSpan> {
        let mut out: Vec<ProseSpan> = Vec::new();
        let mut open = Open::default();
        let mut put = |open: &Open, text: &str, code: bool| {
            if text.is_empty() {
                return;
            }
            let span = ProseSpan { text: String::new(), strong: open.strong > 0, emphasis: open.emphasis > 0, strike: open.strike > 0, code, link: open.links.last().cloned() };
            match out.last_mut() {
                Some(last) if ProseSpan { text: String::new(), ..last.clone() } == span => last.text.push_str(text),
                _ => out.push(ProseSpan { text: text.to_string(), ..span }),
            }
        };
        while let Some(event) = self.events.get(self.at).cloned() {
            match event {
                // an address written out bare leads where it says, as one written as a link does
                Event::Text(text) if open.links.is_empty() => {
                    for piece in addresses().spans(&text) {
                        match piece.kind() {
                            Some(_) => {
                                open.links.push(piece.as_str().to_string());
                                put(&open, piece.as_str(), false);
                                open.links.pop();
                            }
                            None => put(&open, piece.as_str(), false),
                        }
                    }
                }
                Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) | Event::InlineMath(text) | Event::DisplayMath(text) => put(&open, &text, false),
                Event::FootnoteReference(name) => put(&open, &format!("[{name}]"), false),
                Event::Code(text) => put(&open, &text, true),
                Event::SoftBreak | Event::HardBreak => put(&open, "\n", false),
                Event::Start(Tag::Strong) => open.strong += 1,
                Event::Start(Tag::Emphasis) => open.emphasis += 1,
                Event::Start(Tag::Strikethrough) => open.strike += 1,
                // a picture cannot be drawn from here: its description is shown, leading to it
                Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => open.links.push(dest_url.to_string()),
                Event::End(TagEnd::Strong) => open.strong = open.strong.saturating_sub(1),
                Event::End(TagEnd::Emphasis) => open.emphasis = open.emphasis.saturating_sub(1),
                Event::End(TagEnd::Strikethrough) => open.strike = open.strike.saturating_sub(1),
                Event::End(TagEnd::Link | TagEnd::Image) => {
                    open.links.pop();
                }
                _ => break,
            }
            self.at += 1;
        }
        // a line break at either end is the paragraph's own edge
        if let Some(first) = out.first_mut() {
            first.text = first.text.trim_start_matches('\n').to_string();
        }
        if let Some(last) = out.last_mut() {
            last.text = last.text.trim_end_matches('\n').to_string();
        }
        out.retain(|span| !span.text.is_empty());
        out
    }
}

/// The words of some spans, with nothing of how they were marked.
/// The scanner for web addresses written out bare in a run of words: whole addresses that name how they are
/// reached (`https://…`), and nothing else.
fn addresses() -> linkify::LinkFinder {
    let mut finder = linkify::LinkFinder::new();
    finder.kinds(&[linkify::LinkKind::Url]).url_must_have_scheme(true);
    finder
}

/// How many links are shown beside one message.
const LINKS_SHOWN: usize = 4;

/// Where the links in what an agent wrote lead, in order, each once: the ones written as links and the web
/// addresses written out bare, as the Markdown reader finds them. Nothing inside code is a link. A client shows
/// them beside the short form of a message, which leaves them out.
pub fn links(text: &str) -> Vec<String> {
    fn take(spans: &[ProseSpan], out: &mut Vec<String>) {
        for link in spans.iter().filter(|s| !s.code).filter_map(|s| s.link.as_ref()) {
            if crate::links::web(link).is_some() && !out.contains(link) {
                out.push(link.clone());
            }
        }
    }
    fn gather(blocks: &[ProseBlock], out: &mut Vec<String>) {
        for block in blocks {
            match block {
                ProseBlock::Paragraph { spans } | ProseBlock::Heading { spans, .. } => take(spans, out),
                ProseBlock::List { items, .. } => items.iter().for_each(|item| gather(&item.blocks, out)),
                ProseBlock::Quote { blocks } => gather(blocks, out),
                ProseBlock::Table { head, rows, .. } => head.iter().chain(rows.iter().flatten()).for_each(|cell| take(&cell.spans, out)),
                ProseBlock::Code { .. } | ProseBlock::Rule => {}
            }
        }
    }
    let mut out = Vec::new();
    gather(&blocks(text), &mut out);
    out.truncate(LINKS_SHOWN);
    out
}

/// The words of some spans as they are said: a link by its words, and an address written out bare as "a link".
fn words(spans: &[ProseSpan]) -> String {
    spans.iter().map(|span| match &span.link {
        Some(link) if *link == span.text => "a link",
        _ => span.text.as_str(),
    }).collect::<String>()
}

/// `text` with its first letter a capital, as the start of a sentence.
pub fn capital(text: &str) -> String {
    let mut c = text.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// `text` at most `most` characters long: cut, with an ellipsis as its last character, when it is longer.
pub fn clip(text: &str, most: usize) -> String {
    if text.chars().count() <= most {
        return text.to_string();
    }
    format!("{}\u{2026}", text.chars().take(most.saturating_sub(1)).collect::<String>().trim_end())
}

/// The first two sentences of what an agent wrote, as words to say, for when no summary of it was made.
pub fn opening(text: &str) -> String {
    let s = plain(text);
    let mut out = String::new();
    let mut ends = 0;
    for (i, ch) in s.char_indices() {
        out.push(ch);
        if matches!(ch, '.' | '?' | '!') && s[i + ch.len_utf8()..].starts_with(' ') {
            ends += 1;
        }
        if ends == 2 || out.chars().count() >= 300 {
            break;
        }
    }
    out
}

/// What an agent wrote, as words to say: no marks, a table row by row, code named and passed over.
pub fn said(text: &str) -> String {
    let mut out = Vec::new();
    say(&blocks(text), &mut out, true);
    out.join(" ")
}

/// What an agent wrote, as its words on one line with no marks: the same reading as [`said`], with nothing
/// added to them. For showing a message short when no summary of it was made.
pub fn plain(text: &str) -> String {
    let mut out = Vec::new();
    say(&blocks(text), &mut out, false);
    out.join(" ")
}

/// Some words as a sentence of what is said, on one line.
fn sentence(words: String, out: &mut Vec<String>, stop: bool) {
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    if !words.is_empty() {
        out.push(if !stop || words.ends_with(['.', '?', '!', ':', ';', ',']) { words } else { format!("{words}.") });
    }
}

/// Each block as a sentence of its own, so the voice pauses where the page breaks.
fn say(blocks: &[ProseBlock], out: &mut Vec<String>, stop: bool) {
    for block in blocks {
        match block {
            ProseBlock::Paragraph { spans } | ProseBlock::Heading { spans, .. } => sentence(words(spans), out, stop),
            ProseBlock::List { items, .. } => {
                for item in items {
                    say(&item.blocks, out, stop);
                }
            }
            ProseBlock::Quote { blocks } => say(blocks, out, stop),
            ProseBlock::Code { .. } => out.push("There is a block of code here.".into()),
            ProseBlock::Table { head, rows, .. } => {
                for row in rows {
                    // each cell under its column's name: "Day 12 Nov, Customers 9."
                    let cells = row.iter().enumerate().map(|(i, cell)| match head.get(i).map(|h| words(&h.spans)).filter(|h| !h.trim().is_empty()) {
                        Some(name) => format!("{name} {}", words(&cell.spans)),
                        None => words(&cell.spans),
                    });
                    sentence(cells.collect::<Vec<_>>().join(", "), out, stop);
                }
            }
            ProseBlock::Rule => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> ProseSpan {
        ProseSpan { text: text.into(), ..ProseSpan::default() }
    }
    fn paragraph(text: &str) -> ProseBlock {
        ProseBlock::Paragraph { spans: vec![plain(text)] }
    }
    fn items(block: &ProseBlock) -> &[ProseItem] {
        match block {
            ProseBlock::List { items, .. } => items,
            other => panic!("not a list: {other:?}"),
        }
    }

    #[test]
    fn words_keep_their_styles_and_links() {
        let got = blocks("The bug is in `payments/webhook.ts`. **14 customers** were *charged* ~~once~~ twice, see [the run](https://ci.example/1).");
        let ProseBlock::Paragraph { spans } = &got[0] else { panic!("{got:?}") };
        assert_eq!(spans[1], ProseSpan { text: "payments/webhook.ts".into(), code: true, ..ProseSpan::default() });
        assert_eq!(spans[3], ProseSpan { text: "14 customers".into(), strong: true, ..ProseSpan::default() });
        assert!(spans.iter().any(|s| s.emphasis && s.text == "charged") && spans.iter().any(|s| s.strike && s.text == "once"));
        assert_eq!(spans.iter().find(|s| s.link.is_some()).map(|s| (s.text.as_str(), s.link.as_deref())), Some(("the run", Some("https://ci.example/1"))));
        // bold inside a link is both
        let ProseBlock::Paragraph { spans } = &blocks("[**now**](https://a.example)")[0] else { panic!() };
        assert!(spans[0].strong && spans[0].link.is_some());
    }

    #[test]
    fn lists_nest_three_deep_and_number_from_where_they_say() {
        let got = blocks("- one\n  - two\n    - three\n    - three again\n- back\n\n3. third\n4. fourth\n");
        let top = items(&got[0]);
        assert_eq!((top.len(), &top[0].blocks[0], &top[1].blocks[0]), (2, &paragraph("one"), &paragraph("back")));
        let second = items(&top[0].blocks[1]);
        assert_eq!(second[0].blocks[0], paragraph("two"));
        let third = items(&second[0].blocks[1]);
        assert_eq!(third.iter().map(|i| i.blocks[0].clone()).collect::<Vec<_>>(), vec![paragraph("three"), paragraph("three again")]);
        assert!(matches!(&got[1], ProseBlock::List { start: Some(3), items } if items.len() == 2));
        assert!(matches!(&got[0], ProseBlock::List { start: None, .. }));
    }

    #[test]
    fn tasks_say_whether_they_are_done() {
        let got = blocks("- [x] tests pass\n- [ ] deploy\n- plain\n");
        assert_eq!(items(&got[0]).iter().map(|i| i.task).collect::<Vec<_>>(), vec![Some(true), Some(false), None]);
        assert_eq!(items(&got[0])[0].blocks[0], paragraph("tests pass"));
    }

    #[test]
    fn a_list_item_can_hold_code_and_a_heading_can_follow_a_list() {
        let got = blocks("1. Run this:\n\n   ```sh\n   npm test -- payments\n     --watch\n   ```\n\n2. Then wait\n## Next\nDone.");
        let first = &items(&got[0])[0];
        assert_eq!(first.blocks[0], paragraph("Run this:"));
        // the code is as it was written, its own indenting kept
        assert_eq!(first.blocks[1], ProseBlock::Code { language: "sh".into(), text: "npm test -- payments\n  --watch".into() });
        assert_eq!(got[1], ProseBlock::Heading { level: 2, spans: vec![plain("Next")] });
        assert_eq!(got[2], paragraph("Done."));
    }

    #[test]
    fn a_table_keeps_code_and_pipes_in_its_cells() {
        let got = blocks("| Flag | Means | Count |\n|:--|:-:|--:|\n| `a \\| b` | either | 2 |\n| `--dry` | **nothing** sent | 14 |\n");
        let ProseBlock::Table { sides, head, rows } = &got[0] else { panic!("{got:?}") };
        assert_eq!(sides, &vec![ProseSide::Left, ProseSide::Middle, ProseSide::Right]);
        assert_eq!(head.iter().map(|c| words(&c.spans)).collect::<Vec<_>>(), vec!["Flag", "Means", "Count"]);
        assert_eq!(rows[0][0].spans, vec![ProseSpan { text: "a | b".into(), code: true, ..ProseSpan::default() }]);
        assert!(rows[1][1].spans[0].strong && rows.len() == 2 && rows.iter().all(|r| r.len() == 3));
    }

    #[test]
    fn a_fence_never_closed_is_code_to_the_end() {
        // an agent cut off part way through a message
        let got = blocks("Here is the fix:\n\n```rust\nfn main() {\n    let a = 1;");
        assert_eq!(got, vec![paragraph("Here is the fix:"), ProseBlock::Code { language: "rust".into(), text: "fn main() {\n    let a = 1;".into() }]);
        // and bold never closed is the stars it was written with
        assert_eq!(blocks("this is **not closed"), vec![paragraph("this is **not closed")]);
    }

    #[test]
    fn stray_marks_in_plain_words_stay_as_they_were_written() {
        for text in ["2 * 3 * 4 = 24", "snake_case_name and other_name", "a | b | c", "issue #42 and #43", "5 > 3 and a<b", "cost is $5 * 2"] {
            assert_eq!(blocks(text), vec![paragraph(text)], "{text}");
        }
        // a number sign with no space after it is not a heading
        assert_eq!(blocks("#hashtag"), vec![paragraph("#hashtag")]);
    }

    #[test]
    fn html_is_shown_as_the_text_it_is() {
        let got = blocks("before <b onclick=\"x()\">bold</b> after\n\n<script>alert(1)</script>\n\n<details><summary>More</summary>\nhidden\n</details>");
        let all: String = got.iter().map(|b| match b {
            ProseBlock::Paragraph { spans } => words(spans),
            other => panic!("HTML became {other:?}"),
        }).collect::<Vec<_>>().join("\n");
        assert!(all.contains("<b onclick=\"x()\">bold</b>") && all.contains("<script>alert(1)</script>") && all.contains("<summary>More</summary>"), "{all}");
    }

    #[test]
    fn line_breaks_and_long_lines_are_kept() {
        assert_eq!(blocks("first line\nsecond line"), vec![paragraph("first line\nsecond line")]);
        let long = "x".repeat(20_000);
        assert_eq!(blocks(&long), vec![paragraph(&long)]);
        let wide = format!("```\n{long}\n```");
        assert_eq!(blocks(&wide), vec![ProseBlock::Code { language: String::new(), text: long }]);
        assert_eq!(blocks(""), Vec::<ProseBlock>::new());
    }

    #[test]
    fn quotes_hold_blocks_and_a_rule_is_a_rule() {
        let got = blocks("> Refunds over $500 need a second approver.\n> - even small ones on Sundays\n\n---\n\nSend them?");
        let ProseBlock::Quote { blocks: inside } = &got[0] else { panic!("{got:?}") };
        assert_eq!(inside[0], paragraph("Refunds over $500 need a second approver."));
        assert_eq!(items(&inside[1])[0].blocks[0], paragraph("even small ones on Sundays"));
        assert_eq!(&got[1..], &[ProseBlock::Rule, paragraph("Send them?")]);
    }

    #[test]
    fn read_aloud_it_has_no_marks_reads_a_table_by_its_rows_and_passes_over_code() {
        let heard = said("## Duplicate charges\n\nFixed in `payments/webhook.ts`. **14 customers** were charged twice\n\n- [x] tests pass\n- refund them\n\n| Day | Customers |\n|---|--:|\n| 12 Nov | 9 |\n| 13 Nov | 5 |\n\n```sh\nnpm test\n```\n\n> Over $500 needs approval\n\n---\n\nSend them now?");
        assert_eq!(heard, "Duplicate charges. Fixed in payments/webhook.ts. 14 customers were charged twice. tests pass. refund them. Day 12 Nov, Customers 9. Day 13 Nov, Customers 5. There is a block of code here. Over $500 needs approval. Send them now?");
        for mark in ['#', '*', '`', '|', '>', '['] {
            assert!(!heard.contains(mark), "{mark} is in {heard}");
        }
    }

    #[test]
    fn links_are_the_written_ones_and_the_bare_addresses_each_once_and_never_code() {
        let text = "Published: https://claude.ai/artifact/abc123. See [the PR](https://github.com/a/b/pull/7), again https://claude.ai/artifact/abc123 and `http://localhost:3000/x`.\n\n```\ncurl https://in.code/x\n```";
        assert_eq!(links(text), ["https://claude.ai/artifact/abc123", "https://github.com/a/b/pull/7"]);
        assert!(links("No links, only a path /Users/c/x.png and a [note](notes.md)").is_empty());
    }

    #[test]
    fn said_aloud_a_bare_address_is_a_link_and_marks_are_gone() {
        assert_eq!(said("**Done.** See `scripts/check.mjs` and https://example.com/z\n- one"), "Done. See scripts/check.mjs and a link. one.");
        assert_eq!(said("rewrite and/or drop [the doc](https://x.y/z)"), "rewrite and/or drop the doc.");
        assert_eq!(opening("One. Two? Three. Four."), "One. Two?");
        // shown short, an agent's words are its own: nothing is added to them
        assert_eq!(super::plain("the shelves are **stocked**"), "the shelves are stocked");
    }
}
