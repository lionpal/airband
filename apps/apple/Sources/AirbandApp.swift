import AppIntents
import SwiftUI

// One screen: a feed. Everything his agents report or ask arrives in one stream, newest at the bottom, with what
// he said back. Above it, his teams and what each is doing. Below it, the talk button, with quick messages to its
// left and the keyboard to its right. The design is the one on airband.dev.

@main
struct AirbandApp: App {
    @StateObject private var model = RadioModel.shared

    var body: some Scene {
        WindowGroup {
            FeedView()
                .environmentObject(model)
                .onOpenURL { model.pair(with: $0.absoluteString) }
                .tint(Palette.signal)
                .preferredColorScheme(.dark)
        }
    }
}

/// The colours. Dark first. Flat: no gradients or shadows. Each of the three accents means one thing.
enum Palette {
    static let ground = Color(red: 0.043, green: 0.043, blue: 0.047)
    static let ink = Color(red: 0.957, green: 0.957, blue: 0.941)
    /// Green: someone is working; and the answer an agent itself recommends.
    static let signal = Color(red: 0.24, green: 0.86, blue: 0.59)
    /// Amber: someone is waiting on him.
    static let ask = Color(red: 0.96, green: 0.77, blue: 0.32)
    /// Purple: an artifact to open.
    static let art = Color(red: 0.48, green: 0.38, blue: 1.0)
    static let card = Color(red: 0.078, green: 0.090, blue: 0.086)
    static let raised = Color(red: 0.149, green: 0.161, blue: 0.165)
    static let field = Color(red: 0.102, green: 0.106, blue: 0.110)
    static let quiet = Color(red: 0.557, green: 0.557, blue: 0.576)
    static let soft = Color(red: 0.788, green: 0.788, blue: 0.773)
    static let dim = Color(red: 0.333, green: 0.341, blue: 0.353)
    /// Light blue: something to read in full.
    static let read = Color(red: 0.36, green: 0.80, blue: 0.95)
    /// A colour for each team, in the order the tower hands them out: blue, orange, violet, magenta, and last the
    /// two nearest the colours that already mean working and waiting. Checked as a set for telling apart,
    /// including by people who do not see red and green apart; a badge's letters say who it is in any case.
    static let teams: [Color] = [
        Color(red: 0.224, green: 0.529, blue: 0.898), Color(red: 0.851, green: 0.349, blue: 0.149), Color(red: 0.565, green: 0.522, blue: 0.914),
        Color(red: 0.835, green: 0.318, blue: 0.506), Color(red: 0.788, green: 0.522, blue: 0.0), Color(red: 0.098, green: 0.620, blue: 0.439),
    ]
    static func team(_ place: UInt8) -> Color { teams[Int(place) % teams.count] }
}

/// The back of something that sits over the chat: the chat shows through it, blurred and darkened enough that
/// what is on top still reads. `strength` is how much of the dark is laid over the blur; words need more than a button.
struct Frost<S: Shape>: ViewModifier {
    let shape: S
    let strength: Double
    let shown: Bool
    func body(content: Content) -> some View {
        content.background { if shown { shape.fill(.ultraThinMaterial).overlay(shape.fill(Palette.field.opacity(strength))).environment(\.colorScheme, .dark) } }
    }
}
extension View {
    func frosted<S: Shape>(_ shape: S, strength: Double = 0.5, shown: Bool = true) -> some View { modifier(Frost(shape: shape, strength: strength, shown: shown)) }
}

/// Who an agent is, at a glance: two characters on its team's colour.
struct Badge: View {
    let text: String
    let colour: UInt8
    var size: CGFloat = 22

    var body: some View {
        Text(text).font(.system(size: size * 0.5, weight: .bold, design: .rounded)).foregroundStyle(Palette.ground)
            .minimumScaleFactor(0.7).lineLimit(1).padding(.horizontal, 2)
            .frame(width: size, height: size).background(Palette.team(colour), in: RoundedRectangle(cornerRadius: size * 0.3))
            .accessibilityHidden(true)
    }
}

/// What a team or an agent is doing, as a dot: green and pulsing while working, amber when waiting on him, gray
/// and still when it has stopped without finishing, an empty ring when it has finished and has nothing on.
struct StateDot: View {
    let state: String
    @State private var out = false

    private var color: Color {
        switch state {
        case "asking": Palette.ask
        case "working": Palette.signal
        case "stalled": Palette.quiet
        default: Palette.dim
        }
    }

    var body: some View {
        Circle().fill(state == "idle" || state.isEmpty ? Color.clear : color).frame(width: 8, height: 8)
            .overlay { if state == "idle" || state.isEmpty { Circle().strokeBorder(Palette.dim, lineWidth: 1.5) } }
            .overlay {
                if state == "working" {
                    Circle().stroke(color.opacity(out ? 0 : 0.6), lineWidth: 2).scaleEffect(out ? 2.4 : 1)
                        .onAppear { withAnimation(.easeOut(duration: 1.6).repeatForever(autoreverses: false)) { out = true } }
                }
            }
            .accessibilityHidden(true)
    }
}

struct FeedView: View {
    @EnvironmentObject var model: RadioModel
    @State private var draft = ""
    @State private var showSetup = false
    @State private var showAgents = false
    @State private var showScan = false
    @StateObject private var ticker = Ticker()
    @State private var showQuick = false
    @State private var typingOpen = false
    @FocusState private var typing: Bool

    var body: some View {
        VStack(spacing: 0) {
            header
            if !model.paired {
                unpaired
            } else {
                // above the feed, so what a team is doing can hang down over the top of it
                if model.view.rooms.count > 1 || model.only != nil { teams.zIndex(1) }
                if model.view.feed.isEmpty {
                    blank
                    bar
                } else {
                    // the chat runs on under the buttons: they sit over it, frosted, and it fades towards the bottom edge
                    feed.safeAreaInset(edge: .bottom, spacing: 0) {
                        VStack(spacing: 0) {
                            // the chat he already has stays readable with the computer out of reach; this says why it is still
                            if model.connecting {
                                HStack(spacing: 7) {
                                    ProgressView().controlSize(.small).tint(Palette.quiet)
                                    Text("Connecting").font(.system(size: 13, weight: .medium)).foregroundStyle(Palette.soft)
                                }
                                .padding(.horizontal, 12).padding(.vertical, 5).frosted(Capsule())
                            } else if !model.online {
                                Text("Can\u{2019}t reach your computer").font(.system(size: 13, weight: .medium)).foregroundStyle(Palette.ask)
                                    .padding(.horizontal, 12).padding(.vertical, 5).frosted(Capsule())
                            }
                            bar
                        }
                        .frame(maxWidth: .infinity)
                        .background(alignment: .bottom) {
                            LinearGradient(stops: [.init(color: Palette.ground.opacity(0), location: 0), .init(color: Palette.ground.opacity(0.5), location: 0.4), .init(color: Palette.ground.opacity(0.82), location: 1)], startPoint: .top, endPoint: .bottom)
                                .ignoresSafeArea(edges: .bottom).allowsHitTesting(false)
                        }
                    }
                }
            }
        }
        .background(Palette.ground.ignoresSafeArea())
        .foregroundStyle(Palette.ink)
        .sheet(isPresented: $showSetup) { SetupView().environmentObject(model) }
        .fullScreenCover(isPresented: $showScan) { ScanSheet().environmentObject(model) }
        .sheet(isPresented: $showAgents) { AgentsMenu(openSetup: { showAgents = false; showSetup = true }).environmentObject(model).presentationDetents([.medium, .large]) }
        .task { await ticker.run { model.view.rooms } }
        #if DEBUG
        // development only: `-airband.devQuick 1` opens the quick messages, for a screenshot
        .onAppear {
            if UserDefaults.standard.bool(forKey: "airband.devQuick") { showQuick = true }
            if UserDefaults.standard.bool(forKey: "airband.devAccounts") { showSetup = true }
            if UserDefaults.standard.bool(forKey: "airband.devAgents") { showAgents = true }
        }
        #endif
    }

    // MARK: top

    private var header: some View {
        HStack(spacing: 10) {
            if model.paired {
                // everyone, each team and each agent in it, to pick one to look at and talk to
                Button { showAgents = true } label: {
                    Image(systemName: "line.3.horizontal").font(.system(size: 17, weight: .semibold))
                        .frame(width: 40, height: 40).background(Palette.raised, in: Circle())
                }
                .buttonStyle(.plain).accessibilityLabel("Agents")
            }
            Button { showSetup = true } label: {
                HStack(spacing: 10) {
                    Image("Mark").resizable().frame(width: 30, height: 30).clipShape(RoundedRectangle(cornerRadius: 7))
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
                // the headset: whether what arrives is also read aloud. Lit while it is.
                Button { Task { await model.setCalling(!model.calling) } } label: {
                    Image(systemName: "headphones").font(.system(size: 16, weight: .semibold))
                        .foregroundStyle(model.calling ? Palette.ground : Palette.quiet)
                        .frame(width: 40, height: 40).background(model.calling ? Palette.ink : Palette.raised, in: Circle())
                }
                .buttonStyle(.plain).accessibilityLabel(model.calling ? "Stop reading aloud" : "Read aloud")
                .accessibilityAddTraits(model.calling ? .isSelected : [])
            }
        }
        .padding(.horizontal, 12).padding(.top, 10).padding(.bottom, 8)
    }

    /// The teams bar: one cell per project, each showing what its agents are doing. Tap one to see only that team
    /// and keep what he says among them; tap it again for everyone.
    private var teams: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                if let only = model.only {
                    Button { model.only = nil } label: {
                        HStack(spacing: 7) {
                            Text(only).font(.system(size: 14, weight: .semibold))
                            Image(systemName: "xmark").font(.system(size: 11, weight: .bold))
                        }
                        .padding(.horizontal, 14).frame(minHeight: 40)
                        .background(Palette.ink, in: Capsule()).foregroundStyle(Palette.ground)
                    }
                    .buttonStyle(.plain).accessibilityLabel("Showing only \(only). Show everyone.")
                }
                ForEach(model.view.rooms, id: \.name) { room in
                    let on = model.room == room.name
                    Button {
                        model.only = nil
                        model.room = on ? nil : room.name
                        model.replyingTo = nil
                    } label: {
                        HStack(spacing: 7) {
                            StateDot(state: room.state).id(room.state)
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
                    .accessibilityLabel("\(room.name), \(room.state == "asking" ? "waiting on you" : room.state == "stalled" ? "stopped" : room.state)\(room.subagents > 0 ? ", \(room.subagents) subagents" : "")")
                    .accessibilityAddTraits(on ? .isSelected : [])
                    // what this team is doing right now: a small bubble out of its chip, typed out, one team at a time
                    // It hangs from whichever end of the chip keeps it on the screen.
                    .overlay {
                        if ticker.room == room.name {
                            GeometryReader { chip in
                                let (at, screen) = (chip.frame(in: .global), UIScreen.main.bounds.width)
                                let wide = min(300, screen - 44)
                                if at.minX > screen - 24 {
                                    // its chip is off the right of the screen: the bubble stays on screen, at the
                                    // edge, and points off that way
                                    DoingBubble(text: ticker.shown, right: true, tail: .trailing)
                                        .offset(x: screen - 14 - wide - at.minX, y: chip.size.height + 3)
                                } else if at.maxX < 24 {
                                    DoingBubble(text: ticker.shown, right: false, tail: .leading)
                                        .offset(x: 14 - at.minX, y: chip.size.height + 3)
                                } else {
                                    let right = at.midX > screen / 2
                                    DoingBubble(text: ticker.shown, right: right)
                                        .frame(width: chip.size.width, alignment: right ? .trailing : .leading)
                                        .offset(x: right ? -6 : 6, y: chip.size.height + 3)
                                }
                            }
                            .transition(.opacity)
                        }
                    }
                    .zIndex(ticker.room == room.name ? 1 : 0)
                }
            }
            .padding(.horizontal, 8)
        }
        .scrollClipDisabled()
        .animation(.easeOut(duration: 0.18), value: ticker.room)
        .padding(.bottom, 8)
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
                Text("No messages yet").font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.quiet)
            } else {
                Image(systemName: "wifi.slash").font(.system(size: 26, weight: .medium)).foregroundStyle(Palette.dim).padding(.bottom, 4)
                Text("Can\u{2019}t reach your computer").font(.system(size: 19, weight: .semibold))
                (Text("Check it\u{2019}s awake and online.\nThen run ") + Text("airband status").font(.system(size: 15, weight: .medium, design: .monospaced)).foregroundColor(Palette.ink) + Text(" on it."))
                    .font(.system(size: 16)).foregroundStyle(Palette.quiet).multilineTextAlignment(.center).lineSpacing(3)
            }
            Spacer()
        }
        .frame(maxWidth: .infinity).padding(.horizontal, 32)
    }

    // MARK: the feed

    private var feed: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    ForEach(model.view.feed, id: \.id) { m in
                        Group {
                            if m.from == "you" {
                                Mine(m: m)
                            } else if m.needsYou {
                                Question(m: m)
                            } else {
                                Report(m: m)
                            }
                        }
                        .id(m.id)
                    }
                    // what he sent that has not reached the tower yet
                    ForEach(model.view.sending, id: \.id) { o in
                        Text(o.label ?? o.text).font(.system(size: 16, weight: .medium)).foregroundStyle(Palette.ground)
                            .padding(.horizontal, 14).padding(.vertical, 9)
                            .background(Palette.ink.opacity(0.45), in: RoundedRectangle(cornerRadius: 18))
                            .frame(maxWidth: .infinity, alignment: .trailing)
                            .accessibilityLabel("Sending: \(o.label ?? o.text)")
                    }
                    Color.clear.frame(height: 1).id("end")
                }
                .padding(.horizontal, 8).padding(.top, 6).padding(.bottom, 10)
            }
            .defaultScrollAnchor(.bottom)
            .scrollDismissesKeyboard(.interactively)
            .onChange(of: model.view.feed.last?.id) { _, _ in withAnimation(.easeOut(duration: 0.25)) { proxy.scrollTo("end", anchor: .bottom) } }
            .onChange(of: model.view.sending.count) { _, _ in proxy.scrollTo("end", anchor: .bottom) }
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
        if let only = model.only { return "message \(only)" }
        if let room = model.room { return "message \(room)" }
        return "say anything"
    }
    private var listeningNow: Bool { model.listening }
    /// One quiet line above the buttons: who he is answering, or what the radio is doing.
    private var statusLine: String? {
        // who it will go to, whenever that is already settled: the message he picked, or the agent he is looking at
        let aim = model.replyingTo.map { name(of: $0) } ?? model.only
        if model.micOpen { return aim.map { "to \($0) \u{b7} tap to send" } ?? "listening \u{b7} tap to send" }
        if let re = model.replyingTo { return "to \(name(of: re)) \u{b7} tap here to undo" }
        switch model.status {
        case .listening: return "listening \u{b7} tap to send"
        case .sending: return "sending"
        case .speaking(let callsign): return callsign.isEmpty ? "speaking" : callsign
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
            if !model.tray.isEmpty { Tray(sends: !listeningNow && !typingOpen) }
            if showQuick { quickGrid }
            if typingOpen {
                HStack(spacing: 10) {
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
                    Text(statusLine ?? " ").font(.system(size: 14, weight: .medium)).foregroundStyle(statusLine == nil ? Palette.quiet : Palette.soft)
                        .frame(height: 18).padding(.horizontal, 12).padding(.vertical, 4)
                        .frosted(Capsule(), shown: statusLine != nil).accessibilityHidden(statusLine == nil)
                        // a tap lets go of the message he picked: what he says next is placed afresh
                        .onTapGesture { if !model.micOpen { model.replyingTo = nil } }
                } else {
                    // his own words, as he says them: the end of a long sentence stays in view
                    if let aim = model.replyingTo.map({ name(of: $0) }) ?? model.only {
                        Text("to \(aim)").font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.soft)
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
                        if listeningNow { model.cancelTalk() } else { withAnimation(.snappy(duration: 0.2)) { showQuick.toggle() } }
                    } label: { PlusButtonLabel(open: showQuick || listeningNow) }
                        .buttonStyle(.plain).accessibilityLabel(listeningNow ? "Cancel" : "Attach, or quick messages").accessibilityAddTraits(showQuick ? .isSelected : [])
                    Spacer()
                    Button { showQuick = false; Task { await model.talk() } } label: {
                        ZStack {
                            Circle().fill(Palette.signal.opacity(listeningNow ? 0.25 : 0)).frame(width: 112, height: 112)
                            Circle().fill(listeningNow ? Palette.signal : Palette.ink).frame(width: 88, height: 88)
                            // open, it is the send button: nothing goes until he taps it
                            Image(systemName: listeningNow ? "arrow.up" : "mic.fill").font(.system(size: 34, weight: .semibold)).foregroundStyle(Palette.ground)
                        }
                        .frame(width: 112, height: 112)
                    }
                    .buttonStyle(.plain).accessibilityLabel(listeningNow ? "Send what you said" : "Talk")
                    Spacer()
                    Button { showQuick = false; typingOpen = true; typing = true } label: {
                        Image(systemName: "keyboard").font(.system(size: 20, weight: .semibold))
                            .frame(width: 56, height: 56).frosted(Circle())
                    }
                    .buttonStyle(.plain).accessibilityLabel("Type")
                }
                .padding(.horizontal, 18)
            }
        }
        .padding(.top, 6).padding(.bottom, 6)
        .onChange(of: model.replyingTo?.id) { _, id in if id != nil && typingOpen { typing = true } }
    }

    /// The quick messages, as a grid above the buttons. One tap sends one to the message he picked, or to whoever
    /// the tower works out. His own come after the standard ones.
    private var quickGrid: some View {
        let all = model.view.quick
        return VStack(spacing: 8) {
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

/// Who said a message, as the small line above it. Tap it to see only that agent.
private struct Speaker: View {
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage

    var body: some View {
        Button { model.room = nil; model.only = m.from } label: {
            HStack(spacing: 7) {
                if !m.badge.isEmpty { Badge(text: m.badge, colour: m.colour, size: 20) }
                Text(m.station.isEmpty ? m.from : "\(m.from) · \(m.station)").font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.quiet)
            }
        }
        .buttonStyle(.plain).accessibilityLabel("\(m.from). Show only this agent.")
    }
}

/// Something an agent reported: who, one line, and any pictures and links that came with it. Tap it to answer
/// that agent and to see everything it wrote.
struct Report: View {
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage
    @State private var expanded = false
    private var picked: Bool { model.replyingTo?.id == m.id }

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 8) {
                Speaker(m: m)
                if m.stalled { Text("stopped").font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.quiet).padding(.horizontal, 8).padding(.vertical, 2).background(Palette.raised, in: Capsule()) }
            }
            Button {
                expanded.toggle()
                if expanded { model.loadFull(m) }
                model.replyingTo = expanded ? m : (picked ? nil : model.replyingTo)
            } label: {
                Text(expanded ? (model.full[m.id] ?? m.text) : m.text)
                    .font(.system(size: 16)).lineSpacing(3).foregroundStyle(Palette.soft)
                    .multilineTextAlignment(.leading).frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(.plain)
            .accessibilityHint(expanded ? "Show the short version" : "Show everything it wrote, and answer this agent")
            if !m.pictures.isEmpty { PictureStrip(m: m, height: 150).padding(.top, 3) }
            if !m.links.isEmpty { LinkRow(m: m).padding(.top, 3) }
        }
        .padding(.horizontal, 6).padding(.vertical, picked ? 8 : 0)
        .background(picked ? Palette.card : .clear, in: RoundedRectangle(cornerRadius: 16))
        .padding(.horizontal, picked ? -4 : 0)
        .contextMenu {
            Button { UIPasteboard.general.string = model.full[m.id] ?? m.text } label: { Label("Copy", systemImage: "doc.on.doc") }
        }
    }
}

/// A question. While it is open it has an amber edge and its answers as buttons; once answered it folds down to
/// the question alone. Tap the sentence to pick it: what he says or types next is then its answer. The button in
/// the corner opens everything behind the question.
struct Question: View {
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage
    @State private var showContext = false
    private var picked: Bool { model.replyingTo?.id == m.id }

    private var text: AttributedString {
        (try? AttributedString(markdown: m.text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(m.text)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: m.open ? 12 : 6) {
            Speaker(m: m)
            Button { if m.open { model.replyingTo = picked ? nil : m } else { showContext = true } } label: {
                Text(text)
                    .font(.system(size: m.open ? 20 : 16, weight: m.open ? .semibold : .regular)).tracking(m.open ? -0.3 : 0)
                    .foregroundStyle(m.open ? Palette.ink : Palette.soft)
                    .lineSpacing(2).multilineTextAlignment(.leading).frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.trailing, m.open ? 30 : 0)
            }
            .buttonStyle(.plain).accessibilityHint(m.open ? (picked ? "Stop answering this one" : "Answer this one: what you say or type next goes to it") : "Show what was behind this question")
            .accessibilityAddTraits(picked ? .isSelected : [])
            if !m.pictures.isEmpty { PictureStrip(m: m, height: m.open ? 150 : 96) }
            if !m.links.isEmpty { LinkRow(m: m) }
            if m.open { Answers(m: m) }
        }
        .padding(.horizontal, 16).padding(.top, 14).padding(.bottom, m.open ? 14 : 12)
        .background(Palette.card, in: RoundedRectangle(cornerRadius: 24))
        // waiting on him: an amber edge. Picked: a white one, and what he says next is its answer.
        .overlay(RoundedRectangle(cornerRadius: 24).strokeBorder(picked ? Palette.ink : Palette.ask.opacity(0.45), lineWidth: picked ? 2 : (m.open ? 1 : 0)))
        .overlay(alignment: .topTrailing) {
            if m.open {
                Button { showContext = true } label: {
                    Image(systemName: "info").font(.system(size: 13, weight: .bold)).foregroundStyle(Palette.soft)
                        .frame(width: 28, height: 28).background(Palette.raised, in: Circle())
                        .frame(width: 44, height: 44)
                }
                .buttonStyle(.plain).accessibilityLabel("More about this question")
            }
        }
        .sheet(isPresented: $showContext) { ContextView(m: m).environmentObject(model) }
        #if DEBUG
        // development only: `-airband.devContext <callsign>` opens that agent's question, for a screenshot,
        // and `-airband.devPick <callsign>` picks it, as a tap on the sentence would
        .onAppear {
            if UserDefaults.standard.string(forKey: "airband.devContext") == m.from { showContext = true }
            if UserDefaults.standard.string(forKey: "airband.devPick") == m.from, m.open { model.replyingTo = m }
        }
        #endif
    }
}

/// Something he said: on the right, with who it went to.
struct Mine: View {
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage
    /// A long one shows its first lines; a tap opens it whole, and another closes it.
    @State private var whole = false
    private var long: Bool { m.long || m.text.count > 140 || m.text.filter { $0 == "\n" }.count > 4 }

    var body: some View {
        VStack(alignment: .trailing, spacing: 3) {
            if !m.to.isEmpty {
                // who it went to, with that agent's badge when it went to one
                HStack(spacing: 5) {
                    Text("to").font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.quiet)
                    if !m.badge.isEmpty { Badge(text: m.badge, colour: m.colour, size: 16) }
                    Text(m.to.joined(separator: ", ")).font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.quiet).lineLimit(1)
                }
            }
            if !m.pictures.isEmpty { PictureStrip(m: m, height: 120) }
            VStack(alignment: .trailing, spacing: 4) {
                Text(whole ? (model.full[m.id] ?? m.text) : m.text).font(.system(size: 16, weight: .medium)).foregroundStyle(Palette.ground)
                    .lineLimit(whole ? nil : 5)
                if long {
                    Image(systemName: whole ? "chevron.up" : "chevron.down").font(.system(size: 11, weight: .bold)).foregroundStyle(Palette.ground.opacity(0.45))
                }
            }
            .padding(.horizontal, 14).padding(.vertical, 9)
            .background(Palette.ink, in: UnevenRoundedRectangle(topLeadingRadius: 18, bottomLeadingRadius: 18, bottomTrailingRadius: 6, topTrailingRadius: 18))
            .contentShape(Rectangle())
            // hold it: copy it, or send the same words to someone else
            .contextMenu {
                Button { UIPasteboard.general.string = model.full[m.id] ?? m.text } label: { Label("Copy", systemImage: "doc.on.doc") }
                Menu {
                    ForEach(model.view.rooms.flatMap(\.members), id: \.id) { agent in
                        Button(agent.callsign) { model.resend(m, to: agent) }
                    }
                } label: { Label("Send to someone else", systemImage: "arrowshape.turn.up.right") }
            }
            .onTapGesture {
                guard long else { return }
                // what the chat carries of a very long one is its opening: the rest is fetched when he opens it
                if !whole && m.long { model.loadFull(m) }
                withAnimation(.snappy(duration: 0.2)) { whole.toggle() }
            }
        }
        .frame(maxWidth: .infinity, alignment: .trailing).padding(.leading, 50)
        .accessibilityElement(children: .combine).accessibilityLabel("You, to \(m.to.joined(separator: ", ")): \(m.text)")
        .accessibilityAddTraits(long ? .isButton : [])
    }
}

/// His words could not be placed: who are they for? One tap sends them; the cross lets them go.
struct AskCard: View {
    @EnvironmentObject var model: RadioModel
    let ask: DeskAsk

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Who is this for?").font(.system(size: 16, weight: .semibold))
                    Text("\u{201C}\(ask.text)\u{201D}").font(.system(size: 14)).foregroundStyle(Palette.quiet).lineLimit(2)
                }
                Spacer(minLength: 8)
                Button { model.choose(nil) } label: {
                    Image(systemName: "xmark").font(.system(size: 12, weight: .bold)).frame(width: 32, height: 32).background(Palette.raised, in: Circle())
                }
                .buttonStyle(.plain).accessibilityLabel("Send it to nobody")
            }
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(ask.choices, id: \.id) { choice in
                        Button { model.choose(choice) } label: {
                            HStack(spacing: 7) {
                                Badge(text: choice.badge, colour: choice.colour, size: 20)
                                Text(choice.callsign).font(.system(size: 15, weight: .semibold)).lineLimit(1)
                            }
                            .padding(.horizontal, 12).frame(minHeight: 44).background(Palette.raised, in: Capsule())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
        .padding(14)
        .background(Palette.card, in: RoundedRectangle(cornerRadius: 22))
        .overlay(RoundedRectangle(cornerRadius: 22).strokeBorder(Palette.ask.opacity(0.5), lineWidth: 1))
        .padding(.horizontal, 8)
        .transition(.move(edge: .bottom).combined(with: .opacity))
    }
}

/// The answer row under a question: the agent's choices, then a small one that hands the decision back to the agent.
struct Answers: View {
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage
    var after: () -> Void = {}
    /// What the small button sends. The agent reads it as its answer.
    static let yolo = "Do whatever you think is best."

    /// Three answers, or a long one, do not fit side by side: each then gets a row of its own.
    private var stacked: Bool { m.options.count > 2 || m.options.contains { $0.count > 16 } }

    private func choice(_ i: Int) -> some View {
        let text = m.options[i]
        let marked = m.recommended == UInt32(i)
        return Button { model.answer(m, text); after() } label: {
            // a long answer wraps to two lines inside the button, then shrinks; it never touches the edges
            Text(text).font(.system(size: 17, weight: .semibold)).lineLimit(2).minimumScaleFactor(0.75)
                .multilineTextAlignment(.center).lineSpacing(-1).padding(.horizontal, 12).padding(.vertical, 6)
                .frame(maxWidth: .infinity, minHeight: 52)
                // green only for the answer the agent itself recommended; with no recommendation, none is marked
                .background(marked ? Palette.signal : Palette.raised, in: RoundedRectangle(cornerRadius: 16))
                .foregroundStyle(marked ? Palette.ground : Palette.ink)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(marked ? "\(text), recommended" : text)
    }

    private var yolo: some View {
        Button { model.answer(m, Answers.yolo); after() } label: {
            Text("YO\nLO").font(.system(size: 13, weight: .heavy, design: .rounded)).tracking(1).lineSpacing(-2)
                .multilineTextAlignment(.center).foregroundStyle(Palette.ink)
                .frame(width: 52, height: 52)
                .background(Palette.raised, in: RoundedRectangle(cornerRadius: 16))
        }
        .buttonStyle(.plain).accessibilityLabel("Do whatever you think is best")
    }

    /// Answer in his own words: the microphone, aimed at this question. With no choices to tap it is the main
    /// button and says so; beside choices it is a small one.
    private func respond(wide: Bool) -> some View {
        Button { Task { await model.talk(about: m) } } label: {
            HStack(spacing: 8) {
                Image(systemName: model.listening ? "arrow.up" : "mic.fill").font(.system(size: 17, weight: .semibold))
                if wide { Text(model.listening ? "Send" : "Respond").font(.system(size: 17, weight: .semibold)) }
            }
            .foregroundStyle(wide ? Palette.ground : Palette.ink)
            .frame(maxWidth: wide ? .infinity : nil, minHeight: 52).frame(width: wide ? nil : 52)
            .background(wide ? Palette.ink : Palette.raised, in: RoundedRectangle(cornerRadius: 16))
        }
        .buttonStyle(.plain).accessibilityLabel(model.listening ? "Send what you said" : "Answer by voice")
    }

    var body: some View {
        if stacked {
            VStack(spacing: 10) {
                ForEach(m.options.indices.dropLast(), id: \.self) { i in choice(i) }
                HStack(spacing: 10) {
                    choice(m.options.count - 1)
                    if !m.permission { respond(wide: false); yolo }
                }
            }
        } else {
            HStack(spacing: 10) {
                ForEach(m.options.indices, id: \.self) { i in choice(i) }
                // permission to do something on his computer is a yes or a no: it is never handed back to the agent
                if !m.permission { respond(wide: m.options.isEmpty); yolo }
            }
        }
    }
}

/// Everything behind one question: what he and the agent said just before, exactly what the agent wrote, and the
/// same answers, or his own words.
struct ContextView: View {
    @EnvironmentObject var model: RadioModel
    @Environment(\.dismiss) private var dismiss
    let m: ChatMessage
    @State private var draft = ""

    /// The last few things between him and this agent before the question.
    private var before: [ChatMessage] {
        model.thread(before: m)
    }
    private var whole: AttributedString {
        let raw = model.full[m.id] ?? m.text
        return (try? AttributedString(markdown: raw, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(raw)
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text((m.station.isEmpty ? m.from : "\(m.from) · \(m.station)")).font(.system(size: 20, weight: .bold))
                Spacer()
                Button { dismiss() } label: {
                    Image(systemName: "xmark").font(.system(size: 14, weight: .bold))
                        .frame(width: 34, height: 34).background(Palette.raised, in: Circle())
                }
                .buttonStyle(.plain).accessibilityLabel("Close")
            }
            .padding(.horizontal, 20).padding(.top, 20).padding(.bottom, 14)
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        ForEach(before, id: \.id) { b in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(b.from == "you" ? "you" : b.from).font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.quiet)
                                Text(b.text).font(.system(size: 16)).lineSpacing(3).foregroundStyle(Palette.soft)
                            }
                        }
                        if !m.pictures.isEmpty { PictureStrip(m: m, height: 200) }
                        if !m.links.isEmpty { LinkRow(m: m) }
                        Text(whole).font(.system(size: 17)).lineSpacing(4).textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(16).background(Palette.card, in: RoundedRectangle(cornerRadius: 22)).id("question")
                    }
                    .padding(.horizontal, 16).padding(.bottom, 12)
                }
                .onAppear { proxy.scrollTo("question", anchor: .top) }
            }
            VStack(spacing: 10) {
                Answers(m: m, after: { dismiss() })
                HStack(spacing: 10) {
                    TextField("", text: $draft, prompt: Text("your own answer").foregroundColor(Palette.quiet), axis: .vertical)
                        .lineLimit(1...4).font(.system(size: 17)).padding(.horizontal, 18).padding(.vertical, 14)
                        .background(Palette.field, in: RoundedRectangle(cornerRadius: 24))
                        .submitLabel(.send)
                    if !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        Button {
                            model.replyingTo = m
                            model.send(draft)
                            dismiss()
                        } label: {
                            Image(systemName: "arrow.up").font(.system(size: 18, weight: .bold)).foregroundStyle(Palette.ground)
                                .frame(width: 48, height: 48).background(Palette.ink, in: Circle())
                        }
                        .buttonStyle(.plain).accessibilityLabel("Send")
                    }
                }
            }
            .padding(.horizontal, 16).padding(.top, 10).padding(.bottom, 12)
        }
        .background(Palette.ground.ignoresSafeArea())
        .foregroundStyle(Palette.ink)
        .preferredColorScheme(.dark)
        .onAppear { model.loadFull(m) }
    }
}

struct SetupView: View {
    @EnvironmentObject var model: RadioModel
    @Environment(\.dismiss) private var dismiss
    // development only: `-airband.devAccounts 1` opens the accounts page, for a screenshot
    @State private var showAccounts = UserDefaults.standard.bool(forKey: "airband.devAccounts")
    @State private var scanning = false

    var body: some View {
        NavigationStack {
            Form {
                if model.paired {
                    Section {
                        NavigationLink("Accounts", isActive: $showAccounts) { AccountsView().environmentObject(model) }
                    }
                }
                Section(footer: Text("Run ") + Text("airband pair").font(.system(.footnote, design: .monospaced)) + Text(" on each computer, then scan the code it shows or paste its link. Each one adds that computer. Treat a link like a password.")) {
                    Button { scanning = true } label: { Label("Scan code", systemImage: "qrcode.viewfinder").font(.body.weight(.semibold)) }
                    // the other way in, for a link sent to the phone: smaller, under the scan
                    Button("Paste a link instead") {
                        model.pair(with: UIPasteboard.general.string ?? "")
                        if model.problem == nil { dismiss() }
                    }
                    .font(.footnote).foregroundStyle(.secondary)
                    if model.paired {
                        Text(model.view.stations.filter { !$0.isEmpty }.isEmpty ? "Paired" : "Paired: " + model.view.stations.filter { !$0.isEmpty }.joined(separator: ", ")).foregroundStyle(.secondary)
                        Button("Forget all", role: .destructive) { model.forgetAll() }
                    }
                    if let problem = model.problem { Text(problem).foregroundStyle(.red) }
                }
            }
            .fullScreenCover(isPresented: $scanning) { ScanSheet().environmentObject(model) }
            .navigationTitle("Pairing").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
    }
}

/// The message holds something to read whole (the judge said so, and what to call it): a button with that name,
/// which opens exactly what the agent wrote, full screen, the way a picture opens.
struct ReadButton: View {
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage
    @State private var open = false

    var body: some View {
        Button {
            model.loadFull(m)
            open = true
        } label: {
            HStack(spacing: 7) {
                Image(systemName: "doc.text").font(.system(size: 13, weight: .bold))
                Text(m.read ?? "").font(.system(size: 15, weight: .semibold)).lineLimit(1)
            }
            .foregroundStyle(Palette.ground).padding(.horizontal, 14).frame(minHeight: 44)
            .background(Palette.read, in: Capsule())
        }
        .buttonStyle(.plain).accessibilityLabel("Read \(m.read ?? "it")")
        .fullScreenCover(isPresented: $open) { ReaderView(m: m).environmentObject(model) }
    }
}

struct ReaderView: View {
    @EnvironmentObject var model: RadioModel
    @Environment(\.dismiss) private var dismiss
    let m: ChatMessage
    private var listening: Bool { model.listening }
    private var reading: Bool { model.reading == m.id }

    /// What the agent wrote, with its bold, links and code kept and its line breaks as written.
    private var text: AttributedString {
        let raw = model.full[m.id] ?? m.text
        return (try? AttributedString(markdown: raw, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(raw)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                if !m.badge.isEmpty { Badge(text: m.badge, colour: m.colour) }
                VStack(alignment: .leading, spacing: 1) {
                    Text(m.read ?? "").font(.system(size: 17, weight: .semibold)).lineLimit(1)
                    Text(m.from).font(.system(size: 13)).foregroundStyle(Palette.quiet).lineLimit(1)
                }
                Spacer()
                // read it to him: the whole thing, in his computer's voice. Tap again to stop.
                Button { model.readAloud(m) } label: {
                    Image(systemName: reading ? "stop.fill" : "speaker.wave.2.fill").font(.system(size: 14, weight: .bold))
                        .foregroundStyle(reading ? Palette.ground : Palette.ink)
                        .frame(width: 40, height: 40).background(reading ? Palette.ink : Palette.raised, in: Circle())
                }
                .buttonStyle(.plain).accessibilityLabel(reading ? "Stop reading" : "Read it aloud")
                Button { dismiss() } label: {
                    Image(systemName: "xmark").font(.system(size: 14, weight: .bold)).frame(width: 40, height: 40).background(Palette.raised, in: Circle())
                }
                .buttonStyle(.plain).accessibilityLabel("Close")
            }
            .padding(.horizontal, 20).padding(.top, 14).padding(.bottom, 12)
            ScrollView {
                Text(text).font(.system(size: 17)).lineSpacing(5).foregroundStyle(Palette.soft)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 20).padding(.bottom, 150)
                    .textSelection(.enabled)
            }
        }
        // the talk button is here as it is on a picture: he reads it and says what he thinks, and it goes to the
        // agent that wrote it
        .overlay(alignment: .bottom) {
            VStack(spacing: 8) {
                Text(listening ? (model.hearing.isEmpty ? "listening" : model.hearing) : "tell \(m.from)").font(.system(size: 14, weight: .medium)).foregroundStyle(Palette.quiet)
                    .lineLimit(2).truncationMode(.head).multilineTextAlignment(.center).padding(.horizontal, 24)
                Button {
                    model.stopReading()
                    Task { await model.talk(about: m) }
                } label: {
                    Image(systemName: listening ? "arrow.up" : "mic.fill").font(.system(size: 28, weight: .semibold)).foregroundStyle(Palette.ground)
                        .frame(width: 76, height: 76).background(listening ? Palette.signal : Palette.ink, in: Circle())
                }
                .buttonStyle(.plain).accessibilityLabel(listening ? "Send what you said" : "Talk to \(m.from) about this")
            }
            .padding(.top, 26).padding(.bottom, 26).frame(maxWidth: .infinity)
            .background(LinearGradient(colors: [Palette.ground.opacity(0), Palette.ground], startPoint: .top, endPoint: UnitPoint(x: 0.5, y: 0.35)).allowsHitTesting(false))
        }
        .background(Palette.ground.ignoresSafeArea())
        .foregroundStyle(Palette.ink)
        .preferredColorScheme(.dark)
        // with read-aloud on, it starts reading as it opens
        .task {
            guard model.calling, !reading else { return }
            model.readAloud(m)
        }
        .onDisappear { if reading { model.stopReading() } }
    }
}

/// Shows what the working teams are doing, one at a time: picks a team, types its agent's latest step out
/// quickly, holds it for a moment, takes it away, and moves on. Nothing here is kept or read aloud.
@MainActor final class Ticker: ObservableObject {
    @Published var room: String?
    @Published var shown = ""
    private var turn = 0
    /// The last step shown for each team.
    private var told: [String: String] = [:]

    func run(_ rooms: @escaping () -> [DeskRoom]) async {
        while !Task.isCancelled {
            // only what is new: a step a team is still on is shown once, not typed out again and again
            let busy = rooms().filter { $0.doing != nil && told[$0.name] != $0.doing }
            guard !busy.isEmpty, !UIAccessibility.isReduceMotionEnabled else {
                try? await Task.sleep(for: .milliseconds(700))
                continue
            }
            let pick = busy[turn % busy.count]
            turn += 1
            told[pick.name] = pick.doing
            shown = ""
            room = pick.name
            for letter in pick.doing ?? "" {
                shown.append(letter)
                try? await Task.sleep(for: .milliseconds(18))
            }
            try? await Task.sleep(for: .milliseconds(1700))
            room = nil
            try? await Task.sleep(for: .milliseconds(900))
        }
    }
}

/// The bubble itself: small, with a tail up to the chip it comes out of and a caret after the last letter.
struct DoingBubble: View {
    let text: String
    /// It hangs from the chip's right end and grows leftwards, for a chip near the right edge of the screen.
    var right = false
    /// Where its tail is: up to the chip it comes out of, or off the side of the screen its chip has scrolled to.
    var tail: Edge = .top
    @State private var caret = true

    var body: some View {
        // as wide as its words, up to most of a phone's width, whatever the width of the chip above it
        HStack(spacing: 0) {
            if right { Spacer(minLength: 0) }
            bubble
            if !right { Spacer(minLength: 0) }
        }
        .frame(width: min(300, UIScreen.main.bounds.width - 44))
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    private var bubble: some View {
        HStack(spacing: 2) {
            Text(text).font(.system(size: 12, weight: .medium)).foregroundStyle(Palette.soft).lineLimit(1).truncationMode(.tail)
            Rectangle().fill(Palette.signal).frame(width: 1.5, height: 12).opacity(caret ? 1 : 0)
                .onAppear { withAnimation(.linear(duration: 0.01).delay(0.35).repeatForever(autoreverses: true)) { caret = false } }
        }
        .padding(.horizontal, 9).padding(.vertical, 5)
        .background(Color(red: 0.118, green: 0.129, blue: 0.133), in: RoundedRectangle(cornerRadius: 9))
        .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(Color(red: 0.18, green: 0.192, blue: 0.2), lineWidth: 1))
        .overlay(alignment: tail == .top ? (right ? .topTrailing : .topLeading) : (tail == .trailing ? .trailing : .leading)) {
            Rectangle().fill(Color(red: 0.118, green: 0.129, blue: 0.133)).frame(width: 7, height: 7).rotationEffect(.degrees(45))
                .offset(x: tail == .top ? (right ? -12 : 12) : (tail == .trailing ? 3.5 : -3.5), y: tail == .top ? -3.5 : 0)
        }
    }
}

/// Everyone, each team, and each agent in it, as a list to pick from: what he picks is all the feed shows, and
/// who hears what he says next. Each row carries the same dot as the teams bar.
struct AgentsMenu: View {
    @EnvironmentObject var model: RadioModel
    @Environment(\.dismiss) private var dismiss
    let openSetup: () -> Void
    /// The team an agent is being held over, while he drags one.
    @State private var over: String?
    /// The agent a new team is being named for.
    @State private var founding: DeskMember?
    @State private var newTeam = ""

    /// The small line under an agent's name: what it runs on, and the computer it is on when there are several.
    private func under(_ agent: DeskMember) -> String {
        [agent.engine, agent.station].filter { !$0.isEmpty }.joined(separator: " \u{b7} ")
    }

    /// An agent dropped on a team, or on one of its agents, joins that team.
    private func drop(_ ids: [String], on team: String) -> Bool {
        guard let agent = model.view.rooms.flatMap(\.members).first(where: { $0.id == ids.first }) else { return false }
        model.moveAgent(agent, to: team)
        return true
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                row(dot: nil, title: "Everyone", detail: nil, strong: true, on: model.room == nil && model.only == nil, inset: 0) {
                    model.only = nil
                    model.room = nil
                }
                ForEach(model.view.rooms, id: \.name) { room in
                    row(dot: room.state, title: room.name, detail: room.subagents > 0 ? "+\(room.subagents) subagents" : nil, strong: true, on: model.room == room.name && model.only == nil, inset: 0, colour: room.colour) {
                        model.only = nil
                        model.room = room.name
                    }
                    .overlay { if over == room.name { RoundedRectangle(cornerRadius: 14).strokeBorder(Palette.team(room.colour), lineWidth: 2) } }
                    .dropDestination(for: String.self) { ids, _ in drop(ids, on: room.name) } isTargeted: { over = $0 ? room.name : (over == room.name ? nil : over) }
                    .padding(.top, 4)
                    ForEach(room.members, id: \.id) { agent in
                        row(dot: agent.state, title: agent.callsign, detail: agent.doing, strong: false, on: model.only == agent.callsign, inset: 22, colour: room.colour, badge: agent.badge, engine: under(agent)) {
                            model.room = nil
                            model.only = agent.callsign
                        }
                        // hold and drag it onto another team; or hold, and pick the team from the list
                        .draggable(agent.id) {
                            HStack(spacing: 8) {
                                Badge(text: agent.badge, colour: room.colour)
                                Text(agent.callsign).font(.system(size: 16, weight: .medium)).foregroundStyle(Palette.ink)
                            }
                            .padding(.horizontal, 12).padding(.vertical, 8).background(Palette.raised, in: RoundedRectangle(cornerRadius: 12))
                        }
                        .dropDestination(for: String.self) { ids, _ in drop(ids, on: room.name) } isTargeted: { over = $0 ? room.name : (over == room.name ? nil : over) }
                        .contextMenu {
                            ForEach(model.view.rooms.filter { $0.name != room.name }, id: \.name) { other in
                                Button("Move to \(other.name)") { model.moveAgent(agent, to: other.name) }
                            }
                            Button { newTeam = ""; founding = agent } label: { Label("New team", systemImage: "plus") }
                        }
                    }
                }
                Button(action: openSetup) {
                    Text("Accounts and pairing").font(.system(size: 16, weight: .medium)).foregroundStyle(Palette.quiet)
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading).padding(.horizontal, 14)
                }
                .buttonStyle(.plain).padding(.top, 6)
            }
            .padding(.horizontal, 12).padding(.top, 18).padding(.bottom, 10)
        }
        .background(Palette.ground.ignoresSafeArea())
        .foregroundStyle(Palette.ink)
        .preferredColorScheme(.dark)
        .alert("New team", isPresented: Binding(get: { founding != nil }, set: { if !$0 { founding = nil } })) {
            TextField("Name", text: $newTeam).textInputAutocapitalization(.never).autocorrectionDisabled()
            Button("Move") { if let agent = founding { model.moveAgent(agent, to: newTeam) }; founding = nil }
            Button("Cancel", role: .cancel) { founding = nil }
        } message: {
            Text(founding.map { "\($0.callsign) moves to it." } ?? "")
        }
    }

    private func row(dot: String?, title: String, detail: String?, strong: Bool, on: Bool, inset: CGFloat, colour: UInt8? = nil, badge: String? = nil, engine: String = "", pick: @escaping () -> Void) -> some View {
        Button {
            pick()
            model.replyingTo = nil
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
                    if !engine.isEmpty { Text(engine).font(.system(size: 12, weight: .medium)).foregroundStyle(Palette.quiet).lineLimit(1) }
                }
                .layoutPriority(1)
                Spacer(minLength: 8)
                if let detail { Text(detail).font(.system(size: 13)).foregroundStyle(Palette.quiet).lineLimit(1).truncationMode(.tail) }
                if on { Image(systemName: "checkmark").font(.system(size: 14, weight: .bold)).foregroundStyle(Palette.signal) }
            }
            .padding(.leading, 14 + inset).padding(.trailing, 14)
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .background(on ? Palette.card : .clear, in: RoundedRectangle(cornerRadius: 14))
            .contentShape(RoundedRectangle(cornerRadius: 14))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title + (engine.isEmpty ? "" : ", " + engine.replacingOccurrences(of: " \u{b7} ", with: ", ")) + (dot.map { ", " + ($0 == "asking" ? "waiting on you" : $0 == "stalled" ? "stopped" : $0) } ?? ""))
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

/// The accounts his agents run under, on each computer: how much of each is used, and a tap to move every
/// session on that computer to another one. Signing in to a new account is done at the computer.
struct AccountsView: View {
    @EnvironmentObject var model: RadioModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                ForEach(model.accounts, id: \.station) { station in
                    VStack(alignment: .leading, spacing: 10) {
                        if model.accounts.count > 1, !station.station.isEmpty {
                            Text(station.station).font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.quiet).padding(.horizontal, 6)
                        }
                        ForEach(station.accounts, id: \.id) { a in
                            Button { model.useAccount(station.station, a) } label: { row(station.station, a) }
                                .buttonStyle(.plain).allowsHitTesting(!a.live && model.switching == nil) // not disabled: that would fade the one in use
                                .accessibilityLabel(a.live ? "\(a.label), in use" : "Switch to \(a.label)")
                        }
                        if let note = station.note {
                            Text(note).font(.system(size: 14)).foregroundStyle(Palette.quiet).padding(.horizontal, 6)
                        }
                    }
                }
                if let problem = model.accountsProblem {
                    Text(problem).font(.system(size: 15)).foregroundStyle(Color.red).padding(.horizontal, 6)
                }
                if model.accounts.isEmpty {
                    Text(model.accountsLoading ? "Checking…" : "No accounts yet.").font(.system(size: 15)).foregroundStyle(Palette.quiet).padding(.horizontal, 6)
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Palette.ground.ignoresSafeArea())
        .foregroundStyle(Palette.ink)
        .navigationTitle("Accounts").navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                if model.accountsLoading { ProgressView() } else {
                    Button { model.loadAccounts(fresh: true) } label: { Image(systemName: "arrow.clockwise") }.accessibilityLabel("Check again")
                }
            }
        }
        .preferredColorScheme(.dark)
        .onAppear { model.loadAccounts(fresh: false) }
    }

    private func row(_ station: String, _ a: AccountEntry) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Text(a.label).font(.system(size: 17, weight: .semibold)).lineLimit(1).truncationMode(.middle)
                Spacer(minLength: 8)
                if model.switching == station + "|" + a.id {
                    ProgressView()
                } else if a.live {
                    Text("in use").font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.signal)
                }
            }
            if let problem = a.problem {
                Text(problem).font(.system(size: 14)).foregroundStyle(Palette.ask)
            }
            ForEach(a.limits, id: \.name) { limit in
                VStack(alignment: .leading, spacing: 5) {
                    GeometryReader { g in
                        ZStack(alignment: .leading) {
                            Capsule().fill(Palette.raised)
                            Capsule().fill(Self.colour(limit.used)).frame(width: max(6, g.size.width * limit.used))
                        }
                    }
                    .frame(height: 6)
                    HStack(spacing: 6) {
                        Text("\(limit.name) \(Int((limit.used * 100).rounded()))%")
                        if let at = limit.resetsAt {
                            Text("· resets \(Date(timeIntervalSince1970: Double(at) / 1000), format: .dateTime.weekday(.abbreviated).hour().minute())")
                        }
                    }
                    .font(.system(size: 13)).foregroundStyle(Palette.quiet)
                }
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Palette.card, in: RoundedRectangle(cornerRadius: 20))
        .overlay(RoundedRectangle(cornerRadius: 20).stroke(a.live ? Palette.signal.opacity(0.45) : .clear, lineWidth: 1))
        .contentShape(RoundedRectangle(cornerRadius: 20))
    }

    /// Green with room, amber when most of it is gone, red when nearly all.
    private static func colour(_ used: Double) -> Color {
        used >= 0.9 ? .red : used >= 0.75 ? Palette.ask : Palette.signal
    }
}

/// "Hey Siri, talk to Airband", the Action button, or a Shortcut: open the microphone.
struct TalkIntent: AppIntent {
    static var title: LocalizedStringResource = "Talk to Airband"
    static var description = IntentDescription("Opens the microphone so you can speak to your agents.")
    static var openAppWhenRun = false

    @MainActor func perform() async throws -> some IntentResult {
        await RadioModel.shared.talk()
        return .result()
    }
}

struct AirbandShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: TalkIntent(), phrases: ["Talk to \(.applicationName)", "\(.applicationName) radio"], shortTitle: "Talk", systemImageName: "dot.radiowaves.left.and.right")
    }
}
