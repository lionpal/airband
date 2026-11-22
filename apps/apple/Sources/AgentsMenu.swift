import SwiftUI

/// Everyone, each team, and each agent in it, as a list to pick from: what he picks is all the feed shows, and
/// who hears what he says next. Each row carries the same dot as the teams bar.
struct AgentsMenu: View {
    @EnvironmentObject var model: RadioModel
    @Environment(\.dismiss) private var dismiss
    let openSetup: () -> Void
    @State private var makingTeam = false
    /// The team an agent is being held over, while he drags one.
    @State private var over: String?
    /// The agent a new team is being named for.
    @State private var founding: DeskMember?
    /// The team he is renaming, and the name he is typing for it.
    @State private var renaming: DeskRoom?
    @State private var newName = ""
    /// The agent he is about to have its computer take over.
    @State private var taking: DeskMember?
    /// The agent he is about to end.
    @State private var ending: DeskMember?
    @State private var newTeam = ""

    /// The small line under an agent's name, after its model: how hard it thinks, and "terminal" for one the
    /// computer knows runs in a terminal of his (it only gets messages from outside until it is taken over).
    /// One in its own tool's app is not called a terminal. The computer is a small picture at the end of the
    /// row, not more words.
    private func under(_ agent: DeskMember) -> [String] {
        [agent.effort, agent.terminal ? "terminal" : ""].filter { !$0.isEmpty }
    }

    /// An agent dropped on a team, or on one of its agents, joins that team.
    private func drop(_ ids: [String], on room: String) -> Bool {
        guard let agent = ids.first.flatMap({ model.member($0)?.agent }) else { return false }
        model.moveAgent(agent, to: .room(room: room))
        return true
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                row(dot: nil, title: "all agents", detail: nil, strong: true, on: model.room == nil && model.only == nil, inset: 0) {
                    model.look()
                }
                ForEach(model.view.rooms, id: \.id) { room in
                    row(dot: room.state, title: room.name, detail: room.subagents > 0 ? "+\(room.subagents) subagents" : nil, strong: true, on: model.room == room.id && model.only == nil, inset: 0, colour: room.colour) {
                        // a second tap on the one he is looking at goes back to everyone
                        let was = model.room == room.id && model.only == nil
                        model.look(room: was ? nil : room.id)
                    }
                    .contextMenu { Button { newName = room.name; renaming = room } label: { Label("Rename team", systemImage: "pencil") } }
                    .overlay { if over == room.id { RoundedRectangle(cornerRadius: 14).strokeBorder(Palette.team(room.colour), lineWidth: 2) } }
                    // a new agent for this team: one tap starts it where the team's others run
                    .overlay(alignment: .trailing) { NewAgentButton(room: room).padding(.trailing, 6) }
                    .dropDestination(for: String.self) { ids, _ in drop(ids, on: room.id) } isTargeted: { over = $0 ? room.id : (over == room.id ? nil : over) }
                    .padding(.top, 4)
                    ForEach(room.members, id: \.id) { agent in
                        row(dot: agent.state, title: agent.callsign, detail: nil, strong: false, on: model.only == agent.id, inset: 22, reserve: 50, colour: room.colour, badge: agent.badge, lead: agent.model, engine: under(agent), doing: agent.doing) {
                            let was = model.only == agent.id
                            model.look(agent: was ? nil : agent.id)
                        }
                        // which tool it is, and which computer it is on when there are several
                        .overlay(alignment: .trailing) { AgentMarks(agent: agent).padding(.trailing, 12).allowsHitTesting(false) }
                        // hold and drag it onto another team; or hold, and pick the team from the list
                        .draggable(agent.id) {
                            HStack(spacing: 8) {
                                Badge(text: agent.badge, colour: room.colour)
                                Text(agent.callsign).font(.system(size: 16, weight: .medium)).foregroundStyle(Palette.ink)
                            }
                            .padding(.horizontal, 12).padding(.vertical, 8).background(Palette.raised, in: RoundedRectangle(cornerRadius: 12))
                        }
                        .dropDestination(for: String.self) { ids, _ in drop(ids, on: room.id) } isTargeted: { over = $0 ? room.id : (over == room.id ? nil : over) }
                        .contextMenu {
                            ForEach(model.view.rooms.filter { $0.id != room.id && $0.members.contains { $0.station == agent.station } }, id: \.id) { other in
                                Button("Move to \(other.name)") { model.moveAgent(agent, to: .room(room: other.id)) }
                            }
                            Button { newTeam = ""; founding = agent } label: { Label("New team", systemImage: "plus") }
                            if !agent.held {
                                Button { taking = agent } label: { Label("Take over from the terminal", systemImage: "arrow.down.to.line") }
                            }
                            if agent.canStop {
                                Button { model.stop(agent) } label: { Label("Stop what it is doing", systemImage: "stop.fill") }
                            }
                            if agent.ends {
                                Button(role: .destructive) { ending = agent } label: { Label("End this agent", systemImage: "xmark.circle") }
                            }
                        }
                    }
                }
                if model.view.online {
                    Button { makingTeam = true } label: {
                        HStack(spacing: 10) {
                            Image(systemName: "plus").font(.system(size: 15, weight: .semibold))
                            Text("New agent").font(.system(size: 16, weight: .medium))
                            Spacer()
                        }
                        .foregroundStyle(Palette.soft).padding(.horizontal, 14).frame(minHeight: 44)
                    }
                    .buttonStyle(.plain).padding(.top, 6)
                }
            }
            .padding(.horizontal, 12).padding(.top, 18).padding(.bottom, 10)
        }
        .sheet(isPresented: $makingTeam) { NewAgentSheet().environmentObject(model) }
        // Settings stays at the bottom, whatever the list above it is scrolled to
        .safeAreaInset(edge: .bottom, spacing: 0) {
            Button(action: openSetup) {
                HStack(spacing: 10) {
                    Image(systemName: "gearshape").font(.system(size: 17, weight: .medium))
                    Text("Settings").font(.system(size: 17, weight: .medium))
                    Spacer()
                }
                .foregroundStyle(Palette.ink).padding(.horizontal, 26).frame(minHeight: 52)
                .background(Palette.ground)
                .overlay(alignment: .top) { Rectangle().fill(Palette.raised).frame(height: 1) }
            }
            .buttonStyle(.plain)
        }
        .darkPage()
        .takeOverAlert($taking)
        .alert("End \(ending?.callsign ?? "")?", isPresented: $ending.isSet()) {
            Button("End", role: .destructive) { if let agent = ending { model.end(agent) }; ending = nil }
            Button("Cancel", role: .cancel) { ending = nil }
        } message: {
            Text("It finishes and leaves. What it has done stays done.")
        }
        .alert("New team", isPresented: $founding.isSet()) {
            TextField("Name", text: $newTeam).textInputAutocapitalization(.never).autocorrectionDisabled()
            Button("Move") { if let agent = founding { model.moveAgent(agent, to: .new(name: newTeam)) }; founding = nil }
            Button("Cancel", role: .cancel) { founding = nil }
        } message: {
            Text(founding.map { "\($0.callsign) moves to it." } ?? "")
        }
        .alert("Rename team", isPresented: $renaming.isSet()) {
            TextField("Name", text: $newName).textInputAutocapitalization(.never).autocorrectionDisabled()
            Button("Rename") { if let room = renaming { model.renameTeam(room, to: newName) }; renaming = nil }
            Button("Cancel", role: .cancel) { renaming = nil }
        } message: {
            Text("Its agents stay in it, and say the new name.")
        }
    }

    private func row(dot: AgentState?, title: String, detail: String?, strong: Bool, on: Bool, inset: CGFloat, reserve: CGFloat = 0, colour: UInt8? = nil, badge: String? = nil, lead: String = "", engine: [String] = [], doing: String? = nil, pick: @escaping () -> Void) -> some View {
        Button {
            // (every pick goes through look(), which lets go of a message he had picked)
            pick()
            dismiss()
        } label: {
            HStack(spacing: 10) {
                if let dot { StateDot(state: dot).id(dot) }
                // an agent's badge on its team's colour; a team's own row carries the colour as a small mark
                if let colour, let badge { Badge(text: badge, colour: colour) } else if let colour { RoundedRectangle(cornerRadius: 3).fill(Palette.team(colour)).frame(width: 10, height: 10) }
                // the name, with the model and thinking level it runs on small underneath, and the computer it is on
                // when there are several; then what it is doing in what room is left
                VStack(alignment: .leading, spacing: 0) {
                    Text(title).font(.system(size: 17, weight: strong ? .semibold : .regular)).lineLimit(1)
                    if !lead.isEmpty || !engine.isEmpty || doing != nil {
                        // one line, always: the model stays, and while it works what follows gives way to what it
                        // is doing, in green, and comes back when it stops. The row never changes height.
                        HStack(spacing: 0) {
                            if !lead.isEmpty { Text(lead + " \u{b7} ").foregroundStyle(Palette.quiet) }
                            ZStack(alignment: .leading) {
                                Text(engine.joined(separator: " \u{b7} ")).foregroundStyle(Palette.quiet).opacity(doing == nil ? 1 : 0)
                                Text(doing ?? "").foregroundStyle(Palette.signal.opacity(0.9)).opacity(doing == nil ? 0 : 1)
                            }
                        }
                        .font(.system(size: 12, weight: .medium)).lineLimit(1)
                        .animation(.easeInOut(duration: 0.3), value: doing)
                    }
                }
                .layoutPriority(1)
                Spacer(minLength: 8)
                if let detail { Text(detail).font(.system(size: 13)).foregroundStyle(Palette.quiet).lineLimit(1).truncationMode(.tail) }
                if on { Image(systemName: "checkmark").font(.system(size: 14, weight: .bold)).foregroundStyle(Palette.signal) }
            }
            // `reserve`: room kept at the end for marks laid over the row, so the name stops short of them
            .padding(.leading, 14 + inset).padding(.trailing, 14 + reserve)
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .background(on ? Palette.card : .clear, in: RoundedRectangle(cornerRadius: 14))
            .contentShape(RoundedRectangle(cornerRadius: 14))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title + engine.map { ", " + $0 }.joined() + (dot.map { ", " + RadioModel.spoken(state: $0) } ?? ""))
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}
