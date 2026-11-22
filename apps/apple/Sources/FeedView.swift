import SwiftUI

struct FeedView: View {
    @EnvironmentObject var model: RadioModel
    @State private var draft = ""
    @State private var showSetup = false
    /// The first run: shown once, until he has been through it.
    @State private var onboarding = !UserDefaults.standard.bool(forKey: "airband.onboarded")
    @State private var showAgents = false
    @State private var showScan = false
    @StateObject private var ticker = Ticker()
    @State private var showQuick = false
    #if DEBUG
    @State private var showProse = false
    #endif
    @State private var typingOpen = false
    /// Is the end of the chat on screen, and a count that goes up each time he asks to be taken there.
    @State private var atEnd = true
    @State private var toEnd = 0
    /// How tall the buttons over the bottom of the chat are now.
    @State private var barHeight: CGFloat = 0
    @FocusState private var typing: Bool
    /// He has just turned reading aloud on: a line says so for a moment.
    @State private var joined = false
    @State private var joinedNote = 0
    /// The sheet that starts a new agent, opened from the screen that has none yet.
    @State private var showNewAgent = false
    @Environment(\.horizontalSizeClass) private var width
    /// A wide window (an iPad, a Mac): the teams and agents stay open down the left, and the chat has the rest.
    private var wide: Bool { width == .regular && model.paired }

    var body: some View {
        HStack(spacing: 0) {
            if wide {
                AgentsMenu(openSetup: { showSetup = true }).environmentObject(model)
                    .frame(width: 280).background(Palette.card.ignoresSafeArea())
                Rectangle().fill(Palette.raised).frame(width: 1).ignoresSafeArea()
            }
            column
        }
        .background(Palette.ground.ignoresSafeArea())
        .foregroundStyle(Palette.ink)
        .sheet(isPresented: $showSetup) { SetupView().environmentObject(model) }
        .fullScreenCover(isPresented: $onboarding) { OnboardingView().environmentObject(model) }
        // several computers and no main one chosen: he is asked once, before anything can be sent
        .confirmationDialog("Choose your main computer", isPresented: Binding(get: { model.view.chooseMain && !onboarding && !showScan }, set: { _ in }), titleVisibility: .visible) {
            ForEach(model.computers, id: \.self) { name in Button(name) { model.setMainComputer(name) } }
        } message: {
            Text(RadioModel.mainComputerNote + " You can change it in Settings.")
        }
        .fullScreenCover(isPresented: $showScan) { ScanSheet().environmentObject(model) }
        .sheet(isPresented: $showNewAgent) { NewAgentSheet().environmentObject(model) }
        .sheet(isPresented: $showAgents) { AgentsMenu(openSetup: { showAgents = false; showSetup = true }).environmentObject(model).presentationDetents([.medium, .large]) }
        .task { await ticker.run { model.view.rooms } }
    }

    private var column: some View {
        VStack(spacing: 0) {
            header
            if !model.paired {
                unpaired
            } else {
                // above the feed, so what a team is doing can hang down over the top of it
                if model.view.rooms.count > 1 || model.only != nil || model.room != nil { teams.zIndex(1) }
                if model.view.feed.isEmpty {
                    blank
                    bar
                } else {
                    // The chat runs on under the buttons: they sit over it, frosted, and it fades towards the bottom
                    // edge. The room they take is part of the chat itself (an empty stretch as tall as they are, after
                    // the last message), so its end always comes to rest above them however it was scrolled there.
                    feed.overlay(alignment: .bottom) {
                        VStack(spacing: 0) {
                            // the chat he already has stays readable with the computer out of reach; this says why it is still
                            if model.connecting {
                                HStack(spacing: 7) {
                                    ProgressView().controlSize(.small).tint(Palette.quiet)
                                    Text("Connecting").font(.system(size: 13, weight: .medium)).foregroundStyle(Palette.soft)
                                }
                                .padding(.horizontal, 12).padding(.vertical, 5).frosted(Capsule())
                            } else if !model.online || !model.view.away.isEmpty {
                                // a tap tries again at once. With several computers it says which
                                Button { model.reconnect() } label: {
                                    HStack(spacing: 6) {
                                        Text(model.cannotReach).foregroundStyle(Palette.ask)
                                        Image(systemName: "arrow.clockwise").font(.system(size: 11, weight: .bold)).foregroundStyle(Palette.soft)
                                    }
                                    .font(.system(size: 13, weight: .medium)).padding(.horizontal, 12).frame(minHeight: 30).frosted(Capsule())
                                }
                                .buttonStyle(.plain).accessibilityLabel("Cannot reach your computer. Try again.")
                            }
                            bar
                        }
                        .frame(maxWidth: .infinity)
                        .background(GeometryReader { g in Color.clear.preference(key: BarHeight.self, value: g.size.height) })
                        .background(alignment: .bottom) {
                            LinearGradient(stops: [.init(color: Palette.ground.opacity(0), location: 0), .init(color: Palette.ground.opacity(typingOpen || !model.tray.isEmpty ? 0.97 : 0.5), location: typingOpen || !model.tray.isEmpty ? 0.12 : 0.4), .init(color: Palette.ground.opacity(typingOpen || !model.tray.isEmpty ? 1 : 0.82), location: 1)], startPoint: .top, endPoint: .bottom)
                                .ignoresSafeArea(edges: .bottom).allowsHitTesting(false)
                        }
                    }
                    .onPreferenceChange(BarHeight.self) { barHeight = $0 }
                }
            }
        }
        // a question's keyboard button: open the text box, for that question
        .onChange(of: model.typeNow) { _, now in
            guard now else { return }
            model.typeNow = false
            showQuick = false
            typingOpen = true
            typing = true
        }
        #if DEBUG
        // development only: `-airband.devQuick 1` opens the quick messages, for a screenshot
        .onAppear {
            if UserDefaults.standard.bool(forKey: "airband.devQuick") { showQuick = true }
            if UserDefaults.standard.bool(forKey: "airband.devAccounts") || UserDefaults.standard.string(forKey: "airband.devPage") != nil { showSetup = true }
            if UserDefaults.standard.bool(forKey: "airband.devAgents") { showAgents = true }
            // `-airband.devProse YES` opens a message with every kind of formatting in it
            if UserDefaults.standard.bool(forKey: "airband.devProse") { showProse = true }
            // `-airband.devNewAgent YES` opens the sheet that starts a new agent, for a screenshot
            if UserDefaults.standard.string(forKey: "airband.devNewAgent") != nil { showNewAgent = true }
        }
        .sheet(isPresented: $showProse) { ProseSample() }
        #endif
    }

    // MARK: top

    private var header: some View {
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

    /// An agent's badge and its team's colour, by its callsign.
    private func badge(of callsign: String) -> (String, UInt8)? {
        for room in model.view.rooms {
            if let agent = room.members.first(where: { $0.callsign == callsign }) { return (agent.badge, room.colour) }
        }
        return nil
    }

    /// The team he is in: the one he picked, or the team of the agent he is looking at.
    private var team: DeskRoom? {
        if let room = model.room { return model.view.rooms.first { $0.name == room } }
        if let only = model.only { return model.view.rooms.first { $0.members.contains { $0.id == only } } }
        return nil
    }
    /// How many unread things are outside the team he is in: shown on the way back to all of them.
    private var elsewhere: UInt32 { model.view.unread - min(model.view.unread, team?.unread ?? 0) }
    /// What is selected, as the id of its chip: for scrolling the row to it.
    private var chosen: String { model.only.map { "agent:" + $0 } ?? model.room.map { "team:" + $0 } ?? "all" }

    private func chip(_ label: String, on: Bool, dot: AgentState? = nil, working: Bool = false, colour: UInt8? = nil, badge: (String, UInt8)? = nil, unread: UInt32 = 0) -> some View {
        HStack(spacing: 7) {
            if let dot { StateDot(state: dot, working: working).id("\(dot)" + (working ? "+" : "")) }
            if let badge { Badge(text: badge.0, colour: badge.1, size: 20) }
            Text(label).font(.system(size: 14, weight: on ? .semibold : .medium)).lineLimit(1)
                .padding(.bottom, 2).overlay(alignment: .bottom) { if let colour { Capsule().fill(Palette.team(colour)).frame(height: 2) } }
            if !on { Unread(count: unread) }
        }
        .padding(.horizontal, 14).frame(minHeight: 40)
        .background(on ? Palette.ink : Palette.field, in: Capsule()).foregroundStyle(on ? Palette.ground : Palette.ink)
    }

    /// The teams bar: one cell per project, each showing what its agents are doing. Tap one to see only that team
    /// and keep what he says among them; tap it again for everyone.
    private var teams: some View {
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
                    Button { model.look(room: team.name) } label: {
                        chip(team.name, on: model.only == nil, dot: team.state, working: team.members.contains { $0.state == .working }, colour: team.colour, unread: team.unread)
                    }
                    .buttonStyle(.plain).id("team:" + team.name).accessibilityLabel(team.unread > 0 && model.only != nil ? "\(team.name), \(team.unread) unread" : team.name)
                    ForEach(team.members, id: \.id) { agent in
                        let short = agent.number.isEmpty ? agent.callsign : agent.number
                        Button { model.look(room: team.name, agent: model.only == agent.id ? nil : agent.id) } label: {
                            chip(short, on: model.only == agent.id, dot: agent.state, badge: (agent.badge, team.colour), unread: agent.unread)
                        }
                        .buttonStyle(.plain).id("agent:" + agent.id).accessibilityLabel(agent.unread > 0 ? "\(agent.callsign), \(agent.unread) unread" : agent.callsign)
                    }
                } else {
                    Button { model.look() } label: { chip("all", on: true) }
                        .buttonStyle(.plain).id("all").accessibilityLabel("all agents").accessibilityAddTraits(.isSelected)
                }
                ForEach(team == nil ? model.view.rooms : [], id: \.name) { room in
                    let on = model.room == room.name
                    Button {
                        model.look(room: on ? nil : room.name)
                    } label: {
                        HStack(spacing: 7) {
                            StateDot(state: room.state, working: room.members.contains { $0.state == .working }).id("\(room.state)" + (room.members.contains { $0.state == .working } ? "+" : ""))
                            Text(room.name).font(.system(size: 14, weight: on ? .semibold : .medium))
                                .padding(.bottom, 2).overlay(alignment: .bottom) { Capsule().fill(Palette.team(room.colour)).frame(height: 2) }
                            if room.subagents > 0 {
                                Text("+\(room.subagents)").font(.system(size: 12, weight: .medium)).foregroundStyle(on ? Palette.dim : Palette.quiet)
                            }
                        }
                        .padding(.horizontal, 14).frame(minHeight: 40)
                        .background(on ? Palette.ink : Palette.field, in: Capsule()).foregroundStyle(on ? Palette.ground : Palette.ink)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("\(room.name), \(RadioModel.spoken(state: room.state))\(room.subagents > 0 ? ", \(room.subagents) subagents" : "")")
                    .accessibilityAddTraits(on ? .isSelected : [])
                    // what this team is doing right now: a small bubble out of its chip, typed out, one team at a time
                    // It hangs from whichever end of the chip keeps it on the screen.
                    .overlay {
                        if ticker.room == room.name {
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
                    .zIndex(ticker.room == room.name ? 1 : 0)
                    .id("team:" + room.name)
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

    private var unpaired: some View {
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
    private var blank: some View {
        VStack(spacing: 10) {
            Spacer()
            if model.connecting {
                ProgressView().controlSize(.large).tint(Palette.quiet).padding(.bottom, 4)
                Text("Connecting").font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.quiet)
            } else if model.online {
                if model.view.rooms.isEmpty {
                    // paired, and no agent has been heard from: say what makes one appear
                    let tool = model.usedTools.first?.name ?? "your coding agent"
                    Text("You\u{2019}re connected").font(.system(size: 19, weight: .semibold))
                    Text("Start \(tool) in a project on your computer and it shows up here, or start one from here.")
                        .font(.system(size: 16)).foregroundStyle(Palette.quiet).multilineTextAlignment(.center).lineSpacing(3)
                    Button { showNewAgent = true } label: {
                        Text("Start an agent").font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.ground)
                            .padding(.horizontal, 22).frame(minHeight: 50).background(Palette.ink, in: RoundedRectangle(cornerRadius: 16))
                    }
                    .buttonStyle(.plain).padding(.top, 14)
                    Text("Sessions that are already open show up by themselves.").font(.system(size: 13)).foregroundStyle(Palette.quiet).multilineTextAlignment(.center).padding(.top, 18)
                    if let first = model.usedTools.first?.first, !first.isEmpty {
                        Text(first).font(.system(size: 13)).foregroundStyle(Palette.quiet).multilineTextAlignment(.center)
                    }
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

    // MARK: the feed

    private var feed: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    ForEach(model.view.feed, id: \.id) { m in
                        Group {
                            if m.his {
                                Mine(m: m, latest: m.id == model.view.feed.last(where: { $0.his })?.id)
                            } else if m.needsYou {
                                Question(m: m)
                            } else {
                                Report(m: m)
                            }
                        }
                        .id(m.id)
                    }
                    // what he sent that has not reached the tower yet
                    ForEach(model.view.sending, id: \.id) { o in Unsent(o: o) }
                    // the room the buttons take over the chat
                    Color.clear.frame(height: barHeight)
                    Color.clear.frame(height: 1).id("end")
                        // whether the end of the chat is on screen: when it is not, a bar offers the way back to it
                        .onAppear { atEnd = true }
                        .onDisappear { atEnd = false }
                }
                .padding(.horizontal, 8).padding(.top, 6).padding(.bottom, 10)
            }
            .defaultScrollAnchor(.bottom)
            .scrollDismissesKeyboard(.interactively)
            // the one swipe: from the left edge, back, wherever there is a back arrow to go back with
            .simultaneousGesture(DragGesture(minimumDistance: 20).onEnded { g in
                let (dx, dy) = (g.translation.width, g.translation.height)
                guard team != nil, g.startLocation.x < 28, dx > 60, dx > abs(dy) * 2 else { return }
                withAnimation(.snappy(duration: 0.2)) { model.look() }
            })
            .onChange(of: model.view.feed.last?.id) { _, _ in withAnimation(.easeOut(duration: 0.25)) { proxy.scrollTo("end", anchor: .bottom) } }
            .onChange(of: model.view.sending.count) { _, _ in proxy.scrollTo("end", anchor: .bottom) }
            .onChange(of: toEnd) { _, _ in withAnimation(.easeOut(duration: 0.25)) { proxy.scrollTo("end", anchor: .bottom) } }
            // the buttons grew or shrank (the text box opened, a file was added): the end stays in view if it was
            .onChange(of: barHeight) { _, _ in if atEnd { proxy.scrollTo("end", anchor: .bottom) } }
            // a different team or agent, or back to everyone: he lands at the latest of it, not part way up
            .onChange(of: chosen) { _, _ in
                Task {
                    for wait in [40, 220] {
                        try? await Task.sleep(for: .milliseconds(wait))
                        proxy.scrollTo("end", anchor: .bottom)
                    }
                }
            }
            // a long chat of messages of very different heights can open on empty space past its end, because the
            // list only guesses the heights of what it has not drawn: go to the end once it is laid out
            .task {
                for wait in [60, 300] {
                    try? await Task.sleep(for: .milliseconds(wait))
                    proxy.scrollTo("end", anchor: .bottom)
                }
            }
            #if DEBUG
            // development only: `-airband.devBehind 1` scrolls back a little, for a screenshot of the chat under the buttons
            .task {
                guard UserDefaults.standard.bool(forKey: "airband.devBehind") else { return }
                try? await Task.sleep(for: .seconds(4))
                if let m = model.view.feed.dropLast(5).last { proxy.scrollTo(m.id, anchor: .center) }
            }
            #endif
        }
    }

    // MARK: bottom

    /// What the agent that said this is called now: it may have been renamed since he picked the message.
    private func name(of m: ChatMessage) -> String {
        model.view.feed.first { $0.id == m.id }?.from ?? m.from
    }
    private var placeholder: String {
        if let re = model.replyingTo { return "reply to \(name(of: re))" }
        if let only = model.onlyName { return "message \(only)" }
        if let room = model.room { return "message \(room)" }
        return "say anything"
    }
    /// Who the line above the buttons says his words will go to, when it says so: the message he picked or the
    /// agent he is looking at, while the microphone is open or a message is picked.
    private var aimed: String? {
        guard model.micOpen || model.replyingTo != nil else { return nil }
        return model.replyingTo.map { name(of: $0) } ?? model.onlyName
    }
    /// One quiet line above the buttons: who he is answering, or what the radio is doing.
    private var statusLine: String? {
        // who it will go to, whenever that is already settled: the message he picked, or the agent he is looking at
        let aim = model.replyingTo.map { name(of: $0) } ?? model.onlyName
        if model.micOpen { return aim.map { "to \($0) \u{b7} tap to send" } ?? "listening \u{b7} tap to send" }
        if let re = model.replyingTo { return "to \(name(of: re)) \u{b7} tap here to undo" }
        switch model.status {
        case .listening: return "listening \u{b7} tap to send"
        case .sending: return "sending"
        case .speaking(let callsign): return callsign ?? "airband"
        default: return nil
        }
    }

    private var bar: some View {
        VStack(spacing: 10) {
            if let problem = model.problem {
                Button { model.problem = nil } label: { Text(problem).font(.footnote).foregroundStyle(.red).frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 12).padding(.vertical, 7).frosted(RoundedRectangle(cornerRadius: 12)) }
                    .buttonStyle(.plain).padding(.horizontal, 8)
            }
            if let ask = model.view.asking { AskCard(ask: ask) }
            if !model.tray.isEmpty { Tray(sends: !model.listening && !typingOpen) }
            if showQuick { quickGrid }
            if typingOpen {
                HStack(spacing: 10) {
                    // attach, or a quick message, without leaving the keyboard
                    Button { withAnimation(.snappy(duration: 0.2)) { showQuick.toggle() } } label: {
                        Image(systemName: "plus").font(.system(size: 20, weight: .semibold)).rotationEffect(.degrees(showQuick ? 45 : 0))
                            .foregroundStyle(showQuick ? Palette.ground : Palette.ink)
                            .frame(width: 44, height: 54).background(showQuick ? Palette.ink : Palette.field, in: RoundedRectangle(cornerRadius: 22))
                    }
                    .buttonStyle(.plain).accessibilityLabel("Attach, or quick messages")
                    TextField("", text: $draft, prompt: Text(placeholder).foregroundColor(Palette.quiet), axis: .vertical)
                        .lineLimit(1...4).focused($typing).font(.system(size: 17))
                        .padding(.horizontal, 20).frame(minHeight: 54)
                        .frosted(RoundedRectangle(cornerRadius: 27), strength: 0.8)
                    let empty = draft.trimmingCharacters(in: .whitespaces).isEmpty
                    Button {
                        if empty {
                            typing = false
                            typingOpen = false
                        } else {
                            model.send(draft)
                            draft = ""
                        }
                    } label: {
                        Image(systemName: empty ? "mic.fill" : "arrow.up").font(.system(size: 22, weight: .bold))
                            .frame(width: 54, height: 54).background(empty ? Palette.raised : Palette.ink, in: Circle())
                            .foregroundStyle(empty ? Palette.ink : Palette.ground)
                    }
                    .buttonStyle(.plain).accessibilityLabel(empty ? "Back to talking" : "Send")
                }
                .padding(.horizontal, 8)
            } else {
                if let caption = model.caption, model.hearing.isEmpty, !model.micOpen {
                    // what the voice is saying, the words lighting up as it says them
                    TimelineView(.periodic(from: .now, by: 0.06)) { _ in
                        let said = min(model.captionReached(), caption.text.count)
                        // a long message is followed along: the words just said, and the ones coming
                        let from = max(0, said - 70)
                        let shown = String(caption.text.dropFirst(from).prefix(140))
                        (Text(shown.prefix(said - from)).foregroundColor(Palette.ink) + Text(shown.dropFirst(said - from)).foregroundColor(Palette.quiet))
                            .font(.system(size: 15, weight: .medium)).lineLimit(2).multilineTextAlignment(.center)
                            .frame(minHeight: 18).padding(.horizontal, 14).padding(.vertical, 4)
                            .frosted(RoundedRectangle(cornerRadius: 16), strength: 0.8).padding(.horizontal, 8).frame(maxWidth: .infinity)
                    }
                    .accessibilityHidden(true)
                } else if model.hearing.isEmpty {
                    HStack(spacing: 6) {
                        // whoever it will go to, with its badge, when that is settled
                        if let aim = aimed, let badge = badge(of: aim) {
                            Badge(text: badge.0, colour: badge.1, size: 16)
                        }
                        Text(statusLine ?? " ").font(.system(size: 14, weight: .medium)).foregroundStyle(statusLine == nil ? Palette.quiet : Palette.soft)
                    }
                        .frame(height: 18).padding(.horizontal, 12).padding(.vertical, 4)
                        .frosted(Capsule(), shown: statusLine != nil).accessibilityHidden(statusLine == nil)
                        // a tap lets go of the message he picked: what he says next is placed afresh
                        .onTapGesture { if !model.micOpen { model.replyingTo = nil } }
                } else {
                    // his own words, as he says them: the end of a long sentence stays in view
                    if let aim = model.replyingTo.map({ name(of: $0) }) ?? model.onlyName {
                        HStack(spacing: 5) {
                            Text("to")
                            if let badge = badge(of: aim) { Badge(text: badge.0, colour: badge.1, size: 16) }
                            Text(aim)
                        }
                        .font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.soft)
                        .padding(.horizontal, 10).padding(.vertical, 3).frosted(Capsule())
                    }
                    Text("\u{201C}\(model.hearing)\u{201D}").font(.system(size: 17, weight: .medium)).foregroundStyle(Palette.ink)
                        .lineLimit(2).truncationMode(.head).multilineTextAlignment(.center)
                        .frame(minHeight: 18).padding(.horizontal, 14).padding(.vertical, 4)
                        .frosted(RoundedRectangle(cornerRadius: 16), strength: 0.8).padding(.horizontal, 8).frame(maxWidth: .infinity)
                        .animation(.easeOut(duration: 0.12), value: model.hearing)
                        // a tap on his words closes the microphone and puts them in the box, to finish by hand
                        .onTapGesture {
                            draft = model.cancelTalk()
                            typingOpen = true
                            typing = true
                        }
                        .accessibilityLabel("What you said. Tap to edit.").accessibilityAddTraits(.isButton)
                }
                HStack {
                    // +: attach a photo or a video, or send one of the things he says all the time
                    // while he is talking it is the way out: a cross that closes the microphone and sends nothing
                    Button {
                        if model.listening { model.cancelTalk() } else { withAnimation(.snappy(duration: 0.2)) { showQuick.toggle() } }
                    } label: { PlusButtonLabel(open: showQuick || model.listening) }
                        .buttonStyle(.plain).accessibilityLabel(model.listening ? "Cancel" : "Attach, or quick messages").accessibilityAddTraits(showQuick ? .isSelected : [])
                    Spacer(minLength: 0)
                    // Join the channel: what arrives is read aloud, hands free. As large as the microphone beside
                    // it, since talking and listening are the two things this is for. Green while it is on. It
                    // keeps its place while he talks, dimmed: nothing is read over him.
                    Button {
                        let on = !model.calling
                        Task { await model.setCalling(on) }
                        joinedNote += 1
                        withAnimation(.easeOut(duration: 0.2)) { joined = on }
                    } label: {
                        ZStack {
                            if model.calling { Circle().fill(Palette.signal).frame(width: 88, height: 88) } else { Circle().fill(.clear).frame(width: 88, height: 88).frosted(Circle()) }
                            Image(systemName: model.calling ? "speaker.wave.2.fill" : "speaker.slash.fill").font(.system(size: 30, weight: .semibold))
                                .foregroundStyle(model.calling ? Palette.ground : Palette.ink)
                        }
                        .frame(width: 96, height: 112)
                    }
                    .buttonStyle(.plain).disabled(model.listening).opacity(model.listening ? 0.35 : 1)
                    .accessibilityLabel(model.calling ? "Stop reading aloud" : "Read aloud")
                    .accessibilityAddTraits(model.calling ? .isSelected : [])
                    Spacer(minLength: 0)
                    Button { showQuick = false; Task { await model.talk() } } label: {
                        ZStack {
                            Circle().fill(Palette.signal.opacity(model.listening ? 0.25 : 0)).frame(width: 104, height: 104)
                            Circle().fill(model.listening ? Palette.signal : Palette.ink).frame(width: 88, height: 88)
                            // open, it is the send button: nothing goes until he taps it
                            Image(systemName: model.listening ? "arrow.up" : "mic.fill").font(.system(size: 34, weight: .semibold)).foregroundStyle(Palette.ground)
                        }
                        .frame(width: 96, height: 112)
                    }
                    .buttonStyle(.plain).accessibilityLabel(model.listening ? "Send what you said" : "Talk")
                                        Spacer(minLength: 0)
                    HStack(spacing: 8) {
                    if model.listening {
                        // while he talks, this adds a photo or a video to what he is saying
                        AttachButton()
                    } else {
                        Button { showQuick = false; typingOpen = true; typing = true } label: {
                            Image(systemName: "keyboard").font(.system(size: 20, weight: .semibold))
                                .frame(width: 56, height: 56).frosted(Circle())
                        }
                        .buttonStyle(.plain).accessibilityLabel("Type")
                    }
                    }
                }
                .padding(.horizontal, 18)
                // said once when he joins, then gone: what the green button now does
                .overlay(alignment: .top) {
                    if joined {
                        Text("New messages will be read aloud").font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.soft)
                            .padding(.horizontal, 14).frame(height: 28).frosted(Capsule(), strength: 0.9)
                            .offset(y: -30).transition(.opacity)
                            .task(id: joinedNote) {
                                try? await Task.sleep(nanoseconds: 3_000_000_000)
                                withAnimation(.easeOut(duration: 0.4)) { joined = false }
                            }
                    }
                }
            }
        }
        .padding(.top, 6).padding(.bottom, 6)
        // scrolled back: a thin bar at the very bottom takes him to the latest again
        .overlay(alignment: .bottom) {
            if !atEnd, !typingOpen, !model.view.feed.isEmpty {
                Button { toEnd += 1 } label: {
                    HStack(spacing: 5) {
                        Image(systemName: "arrow.down").font(.system(size: 10, weight: .bold))
                        Text("latest").font(.system(size: 12, weight: .semibold))
                    }
                    .foregroundStyle(Palette.soft).padding(.horizontal, 14).frame(height: 24).frosted(Capsule(), strength: 0.8)
                }
                .buttonStyle(.plain).offset(y: 14).transition(.opacity).accessibilityLabel("Go to the latest message")
            }
        }
        .animation(.easeOut(duration: 0.15), value: atEnd)
        .onChange(of: model.replyingTo?.id) { _, id in if id != nil && typingOpen { typing = true } }
    }

    /// The quick messages, as a grid above the buttons. One tap sends one to the message he picked, or to whoever
    /// the tower works out. His own come after the standard ones.
    private var quickGrid: some View {
        let all = model.view.quick
        return VStack(spacing: 8) {
            RecentStrip()
            AttachTile { withAnimation(.snappy(duration: 0.2)) { showQuick = false } }
            // two to a row; an odd one out, usually one of his own, takes the whole row
            ForEach(Array(stride(from: 0, to: all.count, by: 2)), id: \.self) { i in
                HStack(spacing: 8) {
                    ForEach(all[i..<min(i + 2, all.count)], id: \.label) { q in
                        Button {
                            model.quick(q)
                            withAnimation(.snappy(duration: 0.2)) { showQuick = false }
                        } label: {
                            Text(q.label).font(.system(size: 16, weight: .semibold)).lineLimit(1).minimumScaleFactor(0.75)
                                .padding(.horizontal, 10).frame(maxWidth: .infinity, minHeight: 50)
                                .background(Palette.raised, in: RoundedRectangle(cornerRadius: 16))
                        }
                        .buttonStyle(.plain).accessibilityHint(q.text)
                    }
                }
            }
        }
        .padding(10)
        .background(Palette.card, in: RoundedRectangle(cornerRadius: 26))
        .padding(.horizontal, 8)
        .transition(.scale(scale: 0.92, anchor: .bottomLeading).combined(with: .opacity))
    }
}
