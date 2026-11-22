// The top of the chat: its header, the row of teams, and what shows before there is anything to show.
import SwiftUI

extension FeedView {
    // MARK: top

    var header: some View {
        HStack(spacing: 10) {
            if model.paired && !wide {
                // everyone, each team and each agent in it, to pick one to look at and talk to
                Button { showAgents = true } label: {
                    RoundIcon("line.3.horizontal", side: 40, glyph: 17, weight: .semibold)
                }
                .buttonStyle(.plain).accessibilityLabel("Agents")
            }
            Button { showSetup = true } label: {
                HStack(spacing: 10) {
                    // at home, talking straight to the computer: the plane under the waves gives way to a house
                    ZStack {
                        Image("Mark").resizable().opacity(model.view.near ? 0 : 1)
                        if model.view.near { HomeMark() }
                    }
                    .frame(width: 30, height: 30).clipShape(RoundedRectangle(cornerRadius: 7))
                    .animation(.easeInOut(duration: 0.25), value: model.view.near)
                    .accessibilityLabel(model.view.near ? "On your home network" : "")
                    Text("airband").font(.system(size: 26, weight: .bold)).tracking(-0.6).lineLimit(1).fixedSize()
                }
            }
            .buttonStyle(.plain).accessibilityLabel("airband. Pairing.")
            Spacer(minLength: 4)
            if model.paired {
                // Poke: set going again whoever has stopped without finishing. Faint until someone has.
                if model.online { Button { model.poke() } label: {
                    Text("Poke").font(.system(size: 15, weight: .semibold))
                        .padding(.horizontal, 16).frame(minHeight: 40)
                        .background(Palette.raised, in: RoundedRectangle(cornerRadius: 13))
                }
                .buttonStyle(.plain).disabled(!model.view.canPoke).opacity(model.view.canPoke ? 1 : 0.3)
                .accessibilityHint("Tells every agent that has stopped without finishing to keep going") }
                // stop what an agent is doing, when one he can stop is at work in what he is looking at
                let stoppable = model.stoppable
                if stoppable.count == 1 {
                    Button { model.stop(stoppable[0]) } label: { stopLabel }.buttonStyle(.plain).accessibilityLabel("Stop \(stoppable[0].callsign)")
                } else if stoppable.count > 1 {
                    Menu {
                        ForEach(stoppable, id: \.id) { agent in Button("Stop \(agent.callsign)") { model.stop(agent) } }
                    } label: { stopLabel }
                    .accessibilityLabel("Stop an agent")
                }
            }
        }
        .padding(.leading, 8).padding(.trailing, 12).padding(.top, 2).padding(.bottom, 6)
    }

    /// The team he is in: the one he picked, or the team of the agent he is looking at.
    var team: DeskRoom? {
        if model.room != nil { return model.room(model.room) }
        if let only = model.only { return model.member(only)?.team }
        return nil
    }
    /// How many unread things are outside the team he is in: shown on the way back to all of them.
    private var elsewhere: UInt32 { model.view.unread - min(model.view.unread, team?.unread ?? 0) }
    /// What is selected, as the id of its chip: for scrolling the row to it.
    var chosen: String { model.only.map { "agent:" + $0 } ?? model.room.map { "team:" + $0 } ?? "all" }

    private func chip(_ label: String, on: Bool, dot: AgentState? = nil, working: Bool = false, colour: UInt8? = nil, badge: (String, UInt8?)? = nil, unread: UInt32 = 0, subagents: UInt32 = 0) -> some View {
        HStack(spacing: 7) {
            if let dot { StateDot(state: dot, working: working).id("\(dot)" + (working ? "+" : "")) }
            if let badge { Badge(text: badge.0, colour: badge.1, size: 20) }
            Text(label).font(.system(size: 14, weight: on ? .semibold : .medium)).lineLimit(1)
                .padding(.bottom, 2).overlay(alignment: .bottom) { if let colour { Capsule().fill(Palette.team(colour)).frame(height: 2) } }
            if subagents > 0 {
                Text("+\(subagents)").accessibilityLabel(subagents == 1 ? "1 subagent" : "\(subagents) subagents").font(.system(size: 12, weight: .medium)).foregroundStyle(on ? Palette.dim : Palette.quiet)
            }
            if !on { Unread(count: unread) }
        }
        .padding(.horizontal, 14).frame(minHeight: 40)
        .background(on ? Palette.ink : Palette.field, in: Capsule()).foregroundStyle(on ? Palette.ground : Palette.ink)
    }

    /// The teams bar: one cell per project, each showing what its agents are doing. Tap one to see only that team
    /// and keep what he says among them; tap it again for everyone.
    var teams: some View {
        ScrollViewReader { row in
        HStack(spacing: 6) {
        if team != nil {
            // back to all the teams: it stays where it is while the chips beside it scroll
            Button { model.look() } label: {
                Image(systemName: "chevron.left").font(.system(size: 14, weight: .bold)).frame(width: 40, height: 40).background(Palette.field, in: Circle())
            }
            // what the other teams have said since he last looked at them
            .overlay(alignment: .topTrailing) { Unread(count: elsewhere).offset(x: 5, y: -4) }
            .buttonStyle(.plain).accessibilityLabel(elsewhere > 0 ? "All teams, \(elsewhere) unread" : "All teams")
            .padding(.leading, 8).background(Palette.ground).zIndex(2)
        }
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                if let team {
                    // inside a team: the team itself, then its agents
                    Button { model.look(room: team.id) } label: {
                        chip(team.name, on: model.only == nil, dot: team.state, working: team.anyWorking, colour: team.colour, unread: team.unread)
                    }
                    .buttonStyle(.plain).id("team:" + team.id).accessibilityLabel(team.unread > 0 && model.only != nil ? "\(team.name), \(team.unread) unread" : team.name)
                    ForEach(team.members, id: \.id) { agent in
                        let short = agent.number.isEmpty ? agent.callsign : agent.number
                        Button { model.look(room: team.id, agent: model.only == agent.id ? nil : agent.id) } label: {
                            chip(short, on: model.only == agent.id, dot: agent.state, badge: (agent.badge, team.colour), unread: agent.unread)
                        }
                        .buttonStyle(.plain).id("agent:" + agent.id).accessibilityLabel(agent.unread > 0 ? "\(agent.callsign), \(agent.unread) unread" : agent.callsign)
                    }
                } else {
                    Button { model.look() } label: { chip("all", on: true) }
                        .buttonStyle(.plain).id("all").accessibilityLabel("all agents").accessibilityAddTraits(.isSelected)
                }
                ForEach(team == nil ? model.view.rooms : [], id: \.id) { room in
                    let on = model.room == room.id
                    Button {
                        model.look(room: on ? nil : room.id)
                    } label: {
                        chip(room.name, on: on, dot: room.state, working: room.anyWorking, colour: room.colour, subagents: room.subagents)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("\(room.name), \(RadioModel.spoken(state: room.state))\(room.subagents > 0 ? ", \(room.subagents) subagents" : "")")
                    .accessibilityAddTraits(on ? .isSelected : [])
                    // what this team is doing right now: a small bubble out of its chip, typed out, one team at a time
                    // It hangs from whichever end of the chip keeps it on the screen.
                    .overlay {
                        if ticker.room == room.id {
                            GeometryReader { chip in
                                let (at, screen) = (chip.frame(in: .global), UIScreen.main.bounds.width)
                                let wide = min(300, screen - 44)
                                // a chip more than half off the screen counts as off it: the bubble sits at that
                                // edge and points off that way
                                if at.midX > screen - 8 {
                                    // its chip is off the right of the screen: the bubble stays on screen, at the
                                    // edge, and points off that way
                                    DoingBubble(text: ticker.shown, right: true, tail: .trailing)
                                        .offset(x: screen - 14 - wide - at.minX, y: chip.size.height + 3)
                                } else if at.midX < 8 {
                                    DoingBubble(text: ticker.shown, right: false, tail: .leading)
                                        .offset(x: 14 - at.minX, y: chip.size.height + 3)
                                } else {
                                    // under its chip, and never past the edge of the screen: a chip that is partly
                                    // off it has its bubble pulled back in
                                    let right = at.midX > screen / 2
                                    let over = right ? max(0, at.maxX - 6 - (screen - 12)) : max(0, 12 - (at.minX + 6))
                                    DoingBubble(text: ticker.shown, right: right)
                                        .frame(width: chip.size.width, alignment: right ? .trailing : .leading)
                                        .offset(x: right ? -6 - over : 6 + over, y: chip.size.height + 3)
                                }
                            }
                            .transition(.opacity)
                        }
                    }
                    .zIndex(ticker.room == room.id ? 1 : 0)
                    .id("team:" + room.id)
                }
            }
            .padding(.leading, team != nil ? 0 : 8).padding(.trailing, 8)
        }
        .scrollClipDisabled(team == nil)
        }
        .animation(.easeOut(duration: 0.18), value: ticker.room)
        .padding(.bottom, 8)
        // the chip for where he is stays in view, however he got there
        .onChange(of: chosen) { _, now in withAnimation(.easeOut(duration: 0.2)) { row.scrollTo(now, anchor: .center) } }
        }
    }

    var unpaired: some View {
        VStack(spacing: 18) {
            Spacer()
            (Text("Run ") + Text("airband pair").font(.system(size: 16, weight: .medium, design: .monospaced)).foregroundColor(Palette.ink) + Text(" on your computer, then scan the code it shows."))
                .font(.system(size: 17)).multilineTextAlignment(.center).foregroundStyle(Palette.quiet)
            Button { showScan = true } label: {
                Label("Scan code", systemImage: "qrcode.viewfinder").font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.ground)
                    .padding(.horizontal, 26).frame(minHeight: 52).background(Palette.ink, in: RoundedRectangle(cornerRadius: 16))
            }
            .buttonStyle(.plain)
            Button("Paste a link instead") { model.pair(with: UIPasteboard.general.string ?? "") }
                .font(.system(size: 15)).foregroundStyle(Palette.quiet).frame(minHeight: 44)
            if let problem = model.problem { Text(problem).font(.footnote).foregroundStyle(.red) }
            Spacer()
        }
        .padding(32)
    }

    /// Nothing to show, said in the middle of where it would be: the computer cannot be reached and what to try,
    /// or simply that nobody has reported yet.
    var blank: some View {
        VStack(spacing: 10) {
            Spacer()
            if model.connecting {
                ProgressView().controlSize(.large).tint(Palette.quiet).padding(.bottom, 4)
                Text("Connecting").font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.quiet)
            } else if model.online {
                if model.view.rooms.isEmpty {
                    // paired, and no agent has been heard from: say what makes one appear
                    // his tool by name when he uses one; otherwise none is singled out
                    let tool = model.onlyTool?.name ?? "your coding agent"
                    Text("You\u{2019}re connected").font(.system(size: 19, weight: .semibold))
                    Text("Start \(tool) on your computer, or start one here.")
                        .font(.system(size: 16)).foregroundStyle(Palette.quiet).multilineTextAlignment(.center).lineSpacing(3)
                    Button { showNewAgent = true } label: {
                        Text("Start an agent").font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.ground)
                            .padding(.horizontal, 22).frame(minHeight: 50).background(Palette.ink, in: RoundedRectangle(cornerRadius: 16))
                    }
                    .buttonStyle(.plain).padding(.top, 14)
                } else {
                    Text("No messages yet").font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.quiet)
                }
            } else {
                Image(systemName: "wifi.slash").font(.system(size: 26, weight: .medium)).foregroundStyle(Palette.dim).padding(.bottom, 4)
                Text(model.cannotReach).font(.system(size: 19, weight: .semibold))
                (Text("Check it\u{2019}s awake and online.\nThen run ") + Text("airband status").font(.system(size: 15, weight: .medium, design: .monospaced)).foregroundColor(Palette.ink) + Text(" on it."))
                    .font(.system(size: 16)).foregroundStyle(Palette.quiet).multilineTextAlignment(.center).lineSpacing(3)
                Button { model.reconnect() } label: {
                    Label("Try again", systemImage: "arrow.clockwise").font(.system(size: 16, weight: .semibold)).foregroundStyle(Palette.ground)
                        .padding(.horizontal, 20).frame(minHeight: 46).background(Palette.ink, in: RoundedRectangle(cornerRadius: 15))
                }
                .buttonStyle(.plain).padding(.top, 12)
            }
            Spacer()
        }
        .frame(maxWidth: .infinity).padding(.horizontal, 32)
    }

    private var stopLabel: some View {
        RoundIcon("stop.fill", side: 40, glyph: 14, weight: .bold).foregroundStyle(Palette.ink)
    }
}

extension DeskRoom {
    /// Some agent in it is working, whatever the team's own state says.
    var anyWorking: Bool { members.contains { $0.state == .working } }
}

extension DeskMember {
    /// He can stop what it is doing now: it is at work, and run by its computer.
    var canStop: Bool { held && state == .working }
}
