import SwiftUI

/// The + on a team's row: start a new agent in that team, on the computer the team is on. With several coding
/// tools in use, it asks which.
struct NewAgentButton: View {
    @EnvironmentObject var model: RadioModel
    let room: DeskRoom

    var body: some View {
        let tools = model.usedTools
        Group {
            if tools.count > 1 {
                Menu {
                    ForEach(tools, id: \.id) { tool in
                        Button(tool.name) { model.newAgent(in: room.id, tool: tool) }
                    }
                } label: { plus }
            } else {
                Button { model.newAgent(in: room.id) } label: { plus }
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel("New agent in \(room.name)")
    }

    private var plus: some View {
        RoundIcon("plus", side: 34, glyph: 15, weight: .bold).foregroundStyle(Palette.ink)
    }
}
