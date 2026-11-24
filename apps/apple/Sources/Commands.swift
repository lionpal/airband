// The commands and skills of the agent his words go to, as its tool lists them: shown right above the box he types
// in while it holds "/" and the start of a name. The core decides what is offered and in what order
// (`DeskView.commands`); this only shows it.
import SwiftUI

/// The list above the box. A tap puts "/name " in the box, for him to add what it is given.
struct CommandList: View {
    let list: CommandsView
    let pick: (CommandOffer) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(list.title).font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.quiet)
                .padding(.horizontal, 16).padding(.top, 12).padding(.bottom, 6)
            if list.asking && list.items.isEmpty {
                ProgressView().tint(Palette.soft).frame(maxWidth: .infinity, minHeight: 44)
            } else if let note = list.note {
                Text(note).font(.system(size: 15)).foregroundStyle(Palette.soft)
                    .padding(.horizontal, 16).padding(.bottom, 14).frame(maxWidth: .infinity, alignment: .leading)
            }
            if !list.items.isEmpty {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(list.items, id: \.name) { c in
                            Button { pick(c) } label: { row(c) }.buttonStyle(.plain)
                        }
                    }
                }
                .frame(maxHeight: 260)
                .scrollBounceBehavior(.basedOnSize)
            }
        }
        .background(Palette.card, in: RoundedRectangle(cornerRadius: 22))
        .padding(.horizontal, 8)
        .transition(.opacity)
    }

    private func row(_ c: CommandOffer) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text("/" + c.name).font(.system(size: 16, weight: .semibold)).foregroundStyle(Palette.ink).lineLimit(1)
                if let hint = c.hint { Text(hint).font(.system(size: 14)).foregroundStyle(Palette.quiet).lineLimit(1) }
                Spacer(minLength: 8)
                if let kind = c.kind { Text(kind).font(.system(size: 12, weight: .medium)).foregroundStyle(Palette.quiet) }
            }
            if !c.about.isEmpty {
                Text(c.about).font(.system(size: 14)).foregroundStyle(Palette.soft).lineLimit(2)
            }
        }
        .padding(.horizontal, 16).padding(.vertical, 9)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
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
