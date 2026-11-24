// The commands and skills of the agent his words go to, as its tool lists them: shown right above the box he types
// in while it holds "/" and the start of a name. The core decides what is offered and in what order
// (`DeskView.commands`); this only shows it.
import SwiftUI

/// The list above the box. A tap puts "/name " in the box, for him to add what it is given. It is as tall as five of
/// its rows, or as the room above the box when that is less (`most`), and scrolls for the rest.
struct CommandList: View {
    let list: CommandsView
    /// The most it may take, top to bottom: the room the chat has above the box.
    let most: CGFloat
    let pick: (CommandOffer) -> Void
    /// How tall each row it has drawn is, by its place in the list.
    @State private var rows: [Int: CGFloat] = [:]

    private static let shown = 5
    private static let heading: CGFloat = 34

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(list.title).font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.quiet)
                .padding(.horizontal, 16).frame(height: Self.heading, alignment: .bottom).padding(.bottom, 2)
            if list.asking && list.items.isEmpty {
                ProgressView().tint(Palette.soft).frame(maxWidth: .infinity, minHeight: 44)
            } else if let note = list.note {
                Text(note).font(.system(size: 15)).foregroundStyle(Palette.soft)
                    .padding(.horizontal, 16).padding(.bottom, 14).frame(maxWidth: .infinity, alignment: .leading)
            }
            if !list.items.isEmpty {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(list.items.enumerated()), id: \.element.name) { i, c in
                            Button { pick(c) } label: { row(c) }.buttonStyle(.plain)
                                .background(GeometryReader { g in Color.clear.preference(key: RowHeights.self, value: [i: g.size.height]) })
                        }
                    }
                    .padding(.bottom, 6)
                }
                .onPreferenceChange(RowHeights.self) { rows.merge($0) { $1 } }
                .frame(height: height)
                .scrollBounceBehavior(.basedOnSize)
            }
        }
        .background(Palette.card, in: RoundedRectangle(cornerRadius: 22))
        .padding(.horizontal, 8)
        .transition(.opacity)
    }

    /// Five rows, or every row when there are fewer, within the room there is under the heading.
    private var height: CGFloat {
        let count = min(list.items.count, Self.shown)
        let five = (0..<count).reduce(CGFloat(6)) { $0 + (rows[$1] ?? 46) }
        return max(46, min(five, most - Self.heading - 2))
    }

    private func row(_ c: CommandOffer) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            HStack(spacing: 6) {
                Text("/" + c.name).font(.system(size: 16, weight: .semibold)).foregroundStyle(Palette.ink).lineLimit(1)
                if let hint = c.hint { Text(hint).font(.system(size: 14)).foregroundStyle(Palette.quiet).lineLimit(1) }
                Spacer(minLength: 8)
                if let kind = c.kind { Text(kind).font(.system(size: 12, weight: .medium)).foregroundStyle(Palette.quiet) }
            }
            if !c.about.isEmpty {
                Text(c.about).font(.system(size: 13)).foregroundStyle(Palette.soft).lineLimit(1)
            }
        }
        .padding(.horizontal, 16).padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}

/// The heights of the rows a list has drawn, by their place in it.
private struct RowHeights: PreferenceKey {
    static let defaultValue: [Int: CGFloat] = [:]
    static func reduce(value: inout [Int: CGFloat], nextValue: () -> [Int: CGFloat]) { value.merge(nextValue()) { $1 } }
}

/// The tile in the + panel that opens the same list: as if he had typed "/".
struct CommandsTile: View {
    let title: String
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            HStack(spacing: 10) {
                Image(systemName: "slash.circle").font(.system(size: 18, weight: .semibold))
                Text(title).font(.system(size: 16, weight: .semibold))
                Spacer()
            }
            .padding(.horizontal, 16).frame(maxWidth: .infinity, minHeight: 54)
            .background(Palette.raised, in: RoundedRectangle(cornerRadius: 16))
        }
        .buttonStyle(.plain)
    }
}
