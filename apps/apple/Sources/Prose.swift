import SwiftUI

/// What an agent wrote, drawn as it was meant: headings, lists inside lists, tasks, tables, quotes, rules and
/// code, with bold, links and inline code in the words. The core reads the marks (`prose`, in the radio crate)
/// and hands back blocks; nothing here looks at a mark. A wide table or block of code scrolls sideways by
/// itself, and the page never does.
///
/// With `plain` it is the short version in a chat line: the words as they stand, in one piece.
struct Prose: View {
    let text: String
    var size: CGFloat = 16
    var colour: Color = Palette.soft
    var plain = false

    var body: some View {
        if plain {
            Text(text).font(.system(size: size)).lineSpacing(3).foregroundStyle(colour)
                .multilineTextAlignment(.leading).frame(maxWidth: .infinity, alignment: .leading)
        } else {
            ProseBlocks(blocks: Self.read(text), size: size, colour: colour, depth: 0)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// Reading a long message takes a moment and a view is drawn many times: each text is read once.
    private static let kept: NSCache<NSString, Read> = {
        let cache = NSCache<NSString, Read>()
        cache.countLimit = 200
        return cache
    }()
    private final class Read {
        let blocks: [ProseBlock]
        init(_ blocks: [ProseBlock]) { self.blocks = blocks }
    }
    private static func read(_ text: String) -> [ProseBlock] {
        if let had = kept.object(forKey: text as NSString) { return had.blocks }
        let blocks = prose(text: text)
        kept.setObject(Read(blocks), forKey: text as NSString)
        return blocks
    }
}

/// Some blocks, one under another. `depth` is how many lists deep they are.
private struct ProseBlocks: View {
    let blocks: [ProseBlock]
    let size: CGFloat
    let colour: Color
    let depth: Int
    var gap: CGFloat = 12
    /// Sizes follow the text size he has set for his phone.
    @ScaledMetric(relativeTo: .body) private var scale: CGFloat = 1

    var body: some View {
        VStack(alignment: .leading, spacing: gap * scale) {
            ForEach(blocks.indices, id: \.self) { i in block(blocks[i]) }
        }
    }

    @ViewBuilder private func block(_ block: ProseBlock) -> some View {
        switch block {
        case .paragraph(let spans):
            Text(words(spans, size: size)).lineSpacing(size * scale * 0.28).foregroundStyle(colour)
                .multilineTextAlignment(.leading).frame(maxWidth: .infinity, alignment: .leading)
        case .heading(let level, let spans):
            Text(words(spans, size: level <= 2 ? size + 3 : size, weight: .bold)).foregroundStyle(Palette.ink)
                .multilineTextAlignment(.leading).frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, level <= 2 ? 0 : 4 * scale)
                .accessibilityAddTraits(.isHeader)
        case .list(let start, let items):
            VStack(alignment: .leading, spacing: 6 * scale) {
                ForEach(items.indices, id: \.self) { i in
                    HStack(alignment: .firstTextBaseline, spacing: 10 * scale) {
                        mark(items[i], number: start.map { $0 + UInt64(i) }, last: start.map { $0 + UInt64(items.count - 1) })
                        ProseBlocks(blocks: items[i].blocks, size: size, colour: colour, depth: depth + 1, gap: 6)
                    }
                    .accessibilityElement(children: .combine)
                }
            }
        case .code(_, let text):
            ScrollView(.horizontal, showsIndicators: false) {
                Text(text).font(.system(size: (size - 3.5) * scale, design: .monospaced)).foregroundStyle(colour)
                    .lineSpacing(2).fixedSize(horizontal: true, vertical: true).padding(10 * scale)
            }
            .background(Palette.card, in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Palette.raised.opacity(0.6), lineWidth: 1))
        case .quote(let blocks):
            HStack(alignment: .top, spacing: 12 * scale) {
                RoundedRectangle(cornerRadius: 1.5).fill(Palette.dim).frame(width: 3)
                ProseBlocks(blocks: blocks, size: size, colour: Palette.quiet, depth: depth, gap: 8)
            }
            .fixedSize(horizontal: false, vertical: true)
        case .table(let sides, let head, let rows):
            table(sides: sides, head: head, rows: rows)
        case .rule:
            Rectangle().fill(Palette.raised).frame(height: 1).frame(maxWidth: .infinity)
        }
    }

    /// What stands before a list item: its number, a tick box, or a dot (a ring once inside another list).
    @ViewBuilder private func mark(_ item: ProseItem, number: UInt64?, last: UInt64?) -> some View {
        if let done = item.task {
            Image(systemName: done ? "checkmark.square.fill" : "square").font(.system(size: size * scale))
                .foregroundStyle(done ? Palette.signal : Palette.quiet)
                .accessibilityLabel(done ? "Done" : "Not done")
        } else if let number {
            // as wide as the widest number in this list, so the words line up
            Text("\(number).").font(.system(size: size * scale)).monospacedDigit().foregroundStyle(Palette.quiet)
                .frame(minWidth: CGFloat("\(last ?? number).".count) * size * scale * 0.56, alignment: .trailing)
        } else {
            Text(depth == 0 ? "\u{2022}" : "\u{25E6}").font(.system(size: size * scale)).foregroundStyle(Palette.quiet)
                .accessibilityHidden(true)
        }
    }

    /// A table as wide as what is in it. One wider than the screen scrolls sideways.
    private func table(sides: [ProseSide], head: [ProseCell], rows: [[ProseCell]]) -> some View {
        let small = size - 2
        func side(_ column: Int) -> Alignment {
            switch column < sides.count ? sides[column] : .left {
            case .left: return .leading
            case .middle: return .center
            case .right: return .trailing
            }
        }
        func cell(_ cell: ProseCell, _ column: Int, head: Bool) -> some View {
            Text(words(cell.spans, size: small, weight: head ? .semibold : .regular)).foregroundStyle(head ? Palette.ink : colour)
                .fixedSize(horizontal: true, vertical: true)
                .padding(.horizontal, 12 * scale).padding(.vertical, 8 * scale)
                .gridColumnAlignment(side(column) == .trailing ? .trailing : side(column) == .center ? .center : .leading)
        }
        return ScrollView(.horizontal, showsIndicators: false) {
            Grid(alignment: .leading, horizontalSpacing: 0, verticalSpacing: 0) {
                if !head.isEmpty {
                    GridRow { ForEach(head.indices, id: \.self) { c in cell(head[c], c, head: true) } }
                        .background(Palette.card)
                }
                ForEach(rows.indices, id: \.self) { r in
                    Rectangle().fill(Palette.raised).frame(height: 1)
                    GridRow { ForEach(rows[r].indices, id: \.self) { c in cell(rows[r][c], c, head: false) } }
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Palette.raised, lineWidth: 1))
            .padding(1)
        }
    }

    /// Words in their styles, as one piece of text that wraps and can be selected. A link opens.
    private func words(_ spans: [ProseSpan], size: CGFloat, weight: Font.Weight = .regular) -> AttributedString {
        var out = AttributedString()
        for span in spans {
            var piece = AttributedString(span.text)
            var font: Font = span.code
                ? .system(size: (size - 1.5) * scale, weight: weight, design: .monospaced)
                : .system(size: size * scale, weight: span.strong && weight == .regular ? .semibold : weight)
            if span.emphasis { font = font.italic() }
            piece.font = font
            if span.strong { piece.foregroundColor = Palette.ink }
            if span.strike { piece.strikethroughStyle = .single }
            if span.code { piece.backgroundColor = Palette.raised.opacity(0.7) }
            if let address = span.link, let url = URL(string: address) {
                piece.link = url
                piece.foregroundColor = Palette.read
                piece.underlineStyle = .single
            }
            out.append(piece)
        }
        return out
    }
}

#if DEBUG
/// Development only: `-airband.devProse YES` opens a message with one of everything in it, for a screenshot.
struct ProseSample: View {
    static let text = """
    ## Duplicate charges

    The bug is fixed in `payments/webhook.ts`. **14 customers** were charged twice, and *none* ~~three~~ twice over.

    ### What I did

    - Found the retry that ran without checking the charge id
    - Added the check, with two tests
      - one for a retry inside a minute
      - one for a retry a day later
        - and the clock set back
    - [x] All 63 payment tests pass
    - [ ] Refunds not sent yet

    ### Next, in order

    9. Refund the second charge
    10. Email each customer

    | Day | Customers | Amount | Where it came from |
    |:--|--:|--:|:--|
    | 12 Nov | 9 | $801.00 | `webhook` retried by the processor |
    | 13 Nov | 5 | $445.00 | the same, after the first deploy |

    > Refunds over $500 in a day need a second approver.

    ```sh
    npm test -- payments --reporter=dot --run-in-band --coverage --watch=false
    63 passed
    ```

    ---

    Send the refunds now, or show you the list first? [See the run](https://example.com/run/1)
    """

    var body: some View {
        ScrollView {
            Prose(text: Self.text).padding(.horizontal, 16).padding(.vertical, 20).textSelection(.enabled)
        }
        .background(Palette.ground.ignoresSafeArea())
        .preferredColorScheme(.dark)
    }
}
#endif
