import SwiftUI

struct FeedView: View {
    @EnvironmentObject var model: RadioModel
    @State var draft = ""
    @State var showSetup = false
    /// The first run: shown once, until he has been through it.
    @State private var onboarding = !UserDefaults.standard.bool(forKey: "airband.onboarded")
    @State var showAgents = false
    @State var showScan = false
    @StateObject var ticker = Ticker()
    @State var showQuick = false
    #if DEBUG
    @State private var showProse = false
    #endif
    @State var typingOpen = false
    /// Is the end of the chat on screen, and a count that goes up each time he asks to be taken there.
    @State var atEnd = true
    @State var toEnd = 0
    /// How tall the buttons over the bottom of the chat are now.
    @State private var barHeight: CGFloat = 0
    @FocusState var typing: Bool
    /// He has just turned reading aloud on: a line says so for a moment.
    @State var joined = false
    @State var joinedNote = 0
    /// The sheet that starts a new agent, opened from the screen that has none yet.
    @State var showNewAgent = false
    @Environment(\.horizontalSizeClass) var width
    /// A wide window (an iPad, a Mac): the teams and agents stay open down the left, and the chat has the rest.
    var wide: Bool { width == .regular && model.paired }

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
        // several computers and no default one chosen: he is asked once, before anything can be sent. Each is
        // named as the core names it to him.
        // Shown with the computers as they are now, and drawn again when one's name changes while it is up.
        .confirmationDialog("Default computer", isPresented: Binding(get: { model.view.chooseMain && !onboarding && !showScan }, set: { _ in }), titleVisibility: .visible, presenting: model.computers) { computers in
            ForEach(computers, id: \.self) { c in Button { model.setMainComputer(c.id) } label: { Label(c.name, systemImage: RadioModel.symbol(c.picture)) } }
        } message: { _ in
            Text(RadioModel.mainComputerNote)
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
                                .buttonStyle(.plain).accessibilityLabel("\(model.cannotReach) Try again.")
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
            if Dev.flag("devQuick") { showQuick = true }
            if Dev.flag("devAccounts") || Dev.text("devPage") != nil { showSetup = true }
            if Dev.flag("devAgents") { showAgents = true }
            // `-airband.devProse YES` opens a message with every kind of formatting in it
            if Dev.flag("devProse") { showProse = true }
            // `-airband.devNewAgent YES` opens the sheet that starts a new agent, for a screenshot
            if Dev.text("devNewAgent") != nil { showNewAgent = true }
        }
        // `-airband.devSlash typed` or `plus`: once an agent is there, look at it and open its commands, by
        // typing "/" and the start of a name, or from the + panel, for a screenshot of each way in
        .task {
            guard let way = Dev.text("devSlash") else { return }
            while model.view.rooms.flatMap(\.members).isEmpty { try? await Task.sleep(for: .milliseconds(300)) }
            model.look(agent: model.view.rooms.flatMap(\.members)[0].id)
            try? await Task.sleep(for: .seconds(2))
            if way == "plus" {
                showQuick = true
                try? await Task.sleep(for: .seconds(Double(Dev.number("devSlashWait"))))
                openCommands()
            } else {
                typingOpen = true
                typing = true
                draft = "/"
                try? await Task.sleep(for: .seconds(Double(Dev.number("devSlashWait"))))
                draft = Dev.text("devSlashTyped") ?? "/"
            }
        }
        // and once his computers are reached, so it opens with them known
        .onChange(of: model.online) { _, online in
            if online, Dev.text("devNewAgent") != nil {
                showNewAgent = false
                DispatchQueue.main.asyncAfter(deadline: .now() + 3) { showNewAgent = true }
            }
        }
        .sheet(isPresented: $showProse) { ProseSample() }
        #endif
    }

    // MARK: the feed

    var feed: some View {
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
                    // what he said aloud that was not heard, or not sent
                    ForEach(model.view.takes, id: \.id) { t in TakeBanner(take: t) }
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
            .onChange(of: model.view.takes.count) { _, _ in proxy.scrollTo("end", anchor: .bottom) }
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
                guard Dev.flag("devBehind") else { return }
                try? await Task.sleep(for: .seconds(4))
                if let m = model.view.feed.dropLast(5).last { proxy.scrollTo(m.id, anchor: .center) }
            }
            #endif
        }
    }

}

/// How tall the buttons over the bottom of the chat are, told upward by the buttons themselves.
struct BarHeight: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}
