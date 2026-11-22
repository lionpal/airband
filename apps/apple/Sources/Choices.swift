import SwiftUI

/// Where a picker is shown: the same one is the body of a first-run screen and of a Settings page.
enum ChoiceMode {
    /// First run: a choice is noted, and anything it has to fetch is fetched at the end.
    case onboarding
    /// Settings: a change is confirmed, then made at once.
    case settings
}

/// One thing he can choose, as a card: a mark, what it is, one line about it, and when it is the one chosen
/// (or being set up) whatever else it needs underneath.
struct ChoiceCard<Extra: View>: View {
    let title: String
    let subtitle: String
    let chosen: Bool
    var open = false
    /// It costs nothing to use: a small tag says so.
    var free = false
    /// A small picture after the tick, by its name among the app's images: a coding tool's mark.
    var mark: String?
    let tap: () -> Void
    @ViewBuilder var extra: () -> Extra

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Button(action: tap) {
                HStack(spacing: 12) {
                    Image(systemName: chosen ? "checkmark.circle.fill" : "circle").font(.system(size: 22))
                        .foregroundStyle(chosen ? Palette.ink : Palette.dim)
                    if let mark, UIImage(named: mark) != nil {
                        Image(mark).resizable().scaledToFit().frame(width: 24, height: 24).foregroundStyle(Palette.ink)
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text(title).font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.ink)
                        if !subtitle.isEmpty { Text(subtitle).font(.system(size: 14)).foregroundStyle(Palette.soft) }
                    }
                    Spacer(minLength: 0)
                    if free {
                        Text("Free").font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.ink)
                            .padding(.horizontal, 8).frame(minHeight: 22).background(Palette.raised, in: Capsule())
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain).accessibilityAddTraits(chosen ? .isSelected : [])
            if chosen || open { extra() }
        }
        .padding(.horizontal, 14).padding(.vertical, 13)
        .background(Palette.card, in: RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(chosen ? Palette.ink : Palette.raised, lineWidth: 2))
    }
}

extension ChoiceCard where Extra == EmptyView {
    init(title: String, subtitle: String, chosen: Bool, mark: String? = nil, tap: @escaping () -> Void) {
        self.init(title: title, subtitle: subtitle, chosen: chosen, open: false, free: false, mark: mark, tap: tap) { EmptyView() }
    }
}

/// What a service needs before it can be used: its key unless one is already kept, and anything else it is
/// told. "Use it" has the core test the key with its service first (a request that costs nothing); a key that does not pass is not kept.
struct ServiceEntry: View {
    @EnvironmentObject var model: RadioModel
    let choice: Choice
    /// A new key if he typed one, and the settings he changed. Calls back with what is wrong, or nil.
    let use: (String?, [String: String], @escaping (String?) -> Void) -> Void
    @State private var key = ""
    @State private var told: [String: String] = [:]
    @State private var more = false
    @State private var checking = false
    @State private var wrong: String?
    /// He asked to put in another key in place of the one kept. The kept one is never shown.
    @State private var replacing = false
    @FocusState private var typing: Bool
    @Environment(\.reveal) private var reveal

    private var kept: Bool { choice.hasKey && !replacing }
    private var ready: Bool { !checking && !(choice.key && !kept && key.trimmingCharacters(in: .whitespaces).isEmpty) }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if !choice.voices.isEmpty {
                VoiceGrid(choices: choice.voices, chosen: told["voice"] ?? choice.voice) { told["voice"] = $0.name }
            }
            if choice.key {
                if kept {
                    HStack {
                        Text("Uses the key you already gave").font(.system(size: 14)).foregroundStyle(Palette.soft)
                        Spacer()
                        Button("Change key") { replacing = true; typing = true }.font(.system(size: 14, weight: .semibold)).foregroundStyle(Palette.ink).buttonStyle(.plain)
                    }
                } else {
                    SecureField(choice.keyName, text: $key).textContentType(.password).font(.system(size: 16)).focused($typing)
                        .padding(.horizontal, 12).frame(minHeight: 44).background(Palette.field, in: RoundedRectangle(cornerRadius: 10))
                    if !choice.keysAt.isEmpty { Text("Keys are made at \(choice.keysAt)").font(.system(size: 12)).foregroundStyle(Palette.quiet) }
                }
            }
            let asks = choice.asks.filter { choice.voices.isEmpty || $0.field != "voice" }
            if !asks.isEmpty {
                if more {
                    ForEach(asks, id: \.field) { ask in
                        TextField(ask.usual.isEmpty ? ask.question : "\(ask.question): \(ask.usual)", text: Binding(get: { told[ask.field] ?? "" }, set: { told[ask.field] = $0 })).focused($typing)
                            .textInputAutocapitalization(.never).autocorrectionDisabled().font(.system(size: 15))
                            .padding(.horizontal, 12).frame(minHeight: 40).background(Palette.field, in: RoundedRectangle(cornerRadius: 10))
                    }
                } else {
                    Button("More settings") { more = true }.font(.system(size: 14, weight: .medium)).foregroundStyle(Palette.quiet).buttonStyle(.plain)
                }
            }
            Button(action: tryIt) {
                HStack(spacing: 8) { Text("Use it"); if checking { ProgressView().controlSize(.small) } }
                    .font(.system(size: 16, weight: .semibold)).foregroundStyle(Palette.ground)
                    .frame(maxWidth: .infinity, minHeight: 44).background(ready ? Palette.ink : Palette.dim, in: RoundedRectangle(cornerRadius: 12))
            }
            .buttonStyle(.plain).disabled(!ready)
            if let wrong { Text(wrong).font(.system(size: 13)).foregroundStyle(.red) }
        }
        .onChange(of: typing) { _, now in
            // after the keyboard has taken its room
            if now { DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) { reveal(choice.id) } }
        }
        // what came of trying the key is under the button: bring it into view
        .onChange(of: wrong) { _, now in
            if now != nil { DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { reveal(choice.id) } }
        }
        #if DEBUG
        // development only: with `-airband.devOpen <service>`, the key in AIRBAND_DEV_KEY and the settings in
        // AIRBAND_DEV_SETTINGS ("region=eastus") are filled in and "Use it" is pressed, to see the key tried
        .onAppear {
            guard Dev.text("devOpen") == choice.id, let dev = ProcessInfo.processInfo.environment["AIRBAND_DEV_KEY"] else { return }
            key = dev
            for pair in (ProcessInfo.processInfo.environment["AIRBAND_DEV_SETTINGS"] ?? "").split(separator: ",") {
                let parts = pair.split(separator: "=", maxSplits: 1).map(String.init)
                if parts.count == 2 { told[parts[0]] = parts[1]; more = true }
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) { tryIt() }
        }
        #endif
    }

    /// Have the key tried and, if it passes, kept and the service used.
    private func tryIt() {
        (checking, wrong) = (true, nil)
        let fresh = key.trimmingCharacters(in: .whitespaces)
        use(fresh.isEmpty ? nil : fresh, told.filter { !$0.value.trimmingCharacters(in: .whitespaces).isEmpty }) { problem in
            (checking, wrong) = (false, problem)
            if problem == nil { key = ""; replacing = false }
        }
    }
}

/// Bring the card with this id to the top of the scrolling area it is in. Whatever scrolls the choosers
/// (the first run, a Settings page) supplies it.
private struct RevealKey: EnvironmentKey {
    static let defaultValue: (String) -> Void = { _ in }
}
extension EnvironmentValues {
    var reveal: (String) -> Void {
        get { self[RevealKey.self] }
        set { self[RevealKey.self] = newValue }
    }
}

/// Opens the services that are not among the few shown first.
struct MoreServices: View {
    @Binding var shown: Bool
    var body: some View {
        if !shown {
            Button("More services") { withAnimation(.snappy(duration: 0.2)) { shown = true } }
                .font(.system(size: 15, weight: .semibold)).foregroundStyle(Palette.ink).buttonStyle(.plain)
                .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading).padding(.horizontal, 2)
        }
    }
}

/// Fetching a model that runs on the phone, shown inside its card.
private struct Fetching: View {
    let label: String
    let progress: Double?
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let progress { ProgressView(value: progress).tint(Palette.ink) } else { ProgressView().controlSize(.small) }
            Text(label).font(.system(size: 13)).foregroundStyle(Palette.soft)
        }
    }
}

/// The words a job is introduced and asked about with, on the first run and in Settings alike. What can be
/// chosen for it, and how each choice stands, is the core's to say.
extension ChoiceJob {
    /// Its page in Settings.
    var title: String {
        switch self {
        case .reads: return "Summaries"
        case .ears: return "Listening"
        case .voice: return "Voice"
        }
    }
    var headline: String {
        switch self {
        case .reads: return "Long messages, cut down to what you need to read."
        case .ears: return "Talk to your agents."
        case .voice: return "Your agents talk back."
        }
    }
    /// A line under the headline on the first run, where one helps.
    var aside: String? { self == .voice ? "Hear each agent\u{2019}s news read out, even with the screen off." : nil }
    /// What the list is a list of.
    var label: String {
        switch self {
        case .reads: return "Model that writes the summaries"
        case .ears: return "Model that writes down what you say"
        case .voice: return "Model that speaks"
        }
    }
    /// What Settings asks before it changes to `name`: "Download it (65 MB) and listen with it?"
    func asks(_ name: String, download megabytes: UInt32?) -> String {
        let fetch = megabytes.map { "Download \(name) (\($0) MB) and " }
        switch self {
        case .reads: return "Have \(name) write your summaries?"
        case .ears: return fetch.map { $0 + "listen with it?" } ?? "Listen with \(name) on this phone?"
        case .voice: return fetch.map { $0 + "speak with it?" } ?? "Speak with \(name) on this phone?"
        }
    }
}

/// Everything he can choose for one job, as cards: the one chosen is marked, and tapping another chooses it.
/// The same view is the body of that job's first-run screen and of its Settings page. It draws what the core
/// lists (`RadioModel.choices`) and decides nothing itself.
struct ChoicePicker: View {
    @EnvironmentObject var model: RadioModel
    let job: ChoiceJob
    let mode: ChoiceMode
    @State private var open: String?
    @State private var asking: Choice?
    @State private var more = false
    @State private var wrong: String?
    private let tick = Timer.publish(every: 0.25, on: .main, in: .common).autoconnect()

    private var all: [Choice] { model.choices[job] ?? [] }
    private var working: Bool { all.contains { if case .working = $0.state { return true } else { return false } } }

    var body: some View {
        VStack(spacing: 10) {
            ForEach(all.filter { more || !$0.more || $0.chosen }, id: \.id) { c in
                ChoiceCard(title: c.title, subtitle: c.line, chosen: c.chosen, open: open == c.id || c.isWorking, free: c.free) {
                    tapped(c)
                } extra: {
                    extra(c)
                }
                .id(c.id)
            }
            MoreServices(shown: $more)
            if let wrong { Text(wrong).font(.system(size: 13)).foregroundStyle(.red).frame(maxWidth: .infinity, alignment: .leading) }
        }
        .onAppear {
            model.refreshChoices(); model.refreshCatalog()
            #if DEBUG
            // development only: `-airband.devOpen <service>` opens that service's card, as a tap on it would
            if let dev = Dev.text("devOpen") {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { if all.contains(where: { $0.id == dev }) { (open, more) = (dev, true) } }
            }
            #endif
        }
        .onChange(of: model.catalogRev) { _, _ in model.refreshChoices() }
        // while something is being fetched, how far along it is
        .onReceive(tick) { _ in if working { model.refreshChoices() } }
        .confirmationDialog(asking.map { job.asks($0.title, download: $0.megabytesToFetch) } ?? "", isPresented: $asking.isSet(), titleVisibility: .visible) {
            Button(asking?.megabytesToFetch == nil ? "Switch" : "Download") { if let c = asking { choose(c) } }
            Button("Cancel", role: .cancel) {}
        }
    }

    /// One that needs something typed opens for it. Otherwise the first run takes it at once (a model is only
    /// noted, and fetched at the end), and Settings asks first.
    private func tapped(_ c: Choice) {
        if c.chosen && c.megabytesToFetch == nil { return }
        if c.place == .service && (c.key || !c.asks.isEmpty) {
            open = open == c.id ? nil : c.id
        } else if mode == .onboarding {
            choose(c)
        } else {
            asking = c
        }
    }

    private func choose(_ c: Choice, key: String? = nil, settings: [String: String] = [:], done: @escaping (String?) -> Void = { _ in }) {
        wrong = nil
        model.choose(job, c.id, key: key, settings: settings, later: mode == .onboarding) { problem in
            if problem == nil { open = nil } else if open != c.id { wrong = problem }
            done(problem)
        }
    }

    /// What a card shows under its name when it is the one chosen, or is being set up.
    @ViewBuilder private func extra(_ c: Choice) -> some View {
        if open == c.id {
            // what he is typing stays on screen while it is tried
            ServiceEntry(choice: c) { key, settings, done in choose(c, key: key, settings: settings, done: done) }
        } else if case .working(let label, let fraction) = c.state {
            Fetching(label: label, progress: fraction)
        } else if c.chosen {
            if case .failed(let why) = c.state { Text(why).font(.system(size: 13)).foregroundStyle(.red) }
            if case .waiting = c.state, mode == .settings { Text("It will be set up when your computer is paired.").font(.system(size: 13)).foregroundStyle(Palette.quiet) }
            if !c.voices.isEmpty {
                VoiceGrid(choices: c.voices, chosen: c.voice) { voice in model.chooseVoice(voice.name) { wrong = $0 } }
            }
            if job == .voice && c.place == .service || c.key && c.hasKey {
                HStack {
                    if job == .voice && c.place == .service {
                        Button("Hear it") { model.sampleVoice { wrong = $0 } }.font(.system(size: 15, weight: .semibold)).foregroundStyle(Palette.ink).buttonStyle(.plain)
                    }
                    Spacer()
                    if c.key && c.hasKey { Button("Change key") { open = c.id }.font(.system(size: 14, weight: .semibold)).foregroundStyle(Palette.ink).buttonStyle(.plain) }
                }
            }
            if case .stale(let megabytes) = c.state, mode == .settings {
                Button("Get the newer version (\(megabytes) MB)") { choose(c) }.font(.system(size: 14, weight: .semibold)).foregroundStyle(Palette.ink).buttonStyle(.plain)
            }
            if !c.note.isEmpty { Text(c.note).font(.system(size: 12)).foregroundStyle(Palette.quiet) }
            if !c.credit.isEmpty { Text(c.credit).font(.system(size: 11)).foregroundStyle(Palette.quiet) }
        }
    }
}

extension Choice {
    var isWorking: Bool { if case .working = state { return true } else { return false } }
    /// How much choosing it would fetch, when it would fetch anything.
    var megabytesToFetch: UInt32? {
        switch state {
        case .needsDownload(let megabytes), .stale(let megabytes): return megabytes
        default: return nil
        }
    }
}

/// A few voices to choose from: a tile each, with a clip to hear first where there is one. The clips are
/// fetched from where each is published, so they play before anything is downloaded or any key is given.
struct VoiceGrid: View {
    @EnvironmentObject var model: RadioModel
    let choices: [VoiceChoice]
    /// The `name` of the one chosen, or the row's usual one until he picks; the core says which.
    let chosen: String
    let pick: (VoiceChoice) -> Void

    var body: some View {
        LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
            ForEach(choices, id: \.name) { choice in
                let on = chosen == choice.name
                HStack(spacing: 9) {
                    if !choice.preview.isEmpty {
                        Button { model.preview(choice) } label: {
                            Image(systemName: model.previewing == choice.name ? "stop.circle.fill" : "play.circle.fill").font(.system(size: 28))
                                .foregroundStyle(on ? Palette.ground : Palette.ink)
                        }
                        .buttonStyle(.plain).accessibilityLabel(model.previewing == choice.name ? "Stop \(choice.label)" : "Hear \(choice.label)")
                    }
                    Button { pick(choice) } label: {
                        VStack(alignment: .leading, spacing: 0) {
                            Text(choice.label).font(.system(size: 15, weight: .semibold))
                            if !choice.about.isEmpty { Text(choice.about).font(.system(size: 12)).lineLimit(1).minimumScaleFactor(0.8) }
                        }
                        .foregroundStyle(on ? Palette.ground : Palette.ink).frame(maxWidth: .infinity, minHeight: 30, alignment: .leading).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain).accessibilityAddTraits(on ? .isSelected : [])
                }
                .padding(.horizontal, 10).padding(.vertical, 9)
                .background(on ? Palette.ink : Palette.raised, in: RoundedRectangle(cornerRadius: 12))
            }
        }
    }
}

/// One Settings page: the same headline and picker as its onboarding screen.
struct ChoicePage<Picker: View>: View {
    let title: String
    let headline: String
    @ViewBuilder var picker: () -> Picker

    var body: some View {
        ScrollViewReader { page in
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text(headline).font(.system(size: 26, weight: .bold)).tracking(-0.5).foregroundStyle(Palette.ink).fixedSize(horizontal: false, vertical: true)
                    picker()
                }
                .padding(.horizontal, 20).padding(.vertical, 16).frame(maxWidth: 560).frame(maxWidth: .infinity)
            }
            .environment(\.reveal) { id in withAnimation(.snappy(duration: 0.25)) { page.scrollTo(id, anchor: .top) } }
        }
        .background(Palette.ground.ignoresSafeArea())
        .navigationTitle(title).navigationBarTitleDisplayMode(.inline)
        .preferredColorScheme(.dark)
    }
}

/// The coding tools he uses: one is chosen on first run (`single`), and more are added in Settings. The first
/// in his list is the one he uses most.
struct ToolsPicker: View {
    @EnvironmentObject var model: RadioModel
    /// First run: choosing one replaces the choice. Settings: each is added or taken away, and one always stays.
    let single: Bool

    var body: some View {
        VStack(spacing: 10) {
            ForEach(codingTools(), id: \.id) { tool in
                let on = model.tools.contains(tool.id)
                ChoiceCard(title: tool.name, subtitle: tool.maker, chosen: on, mark: "harness-\(tool.id)") {
                    if single {
                        model.tools = [tool.id]
                    } else if on {
                        if model.tools.count > 1 { model.tools.removeAll { $0 == tool.id } }
                    } else {
                        model.tools.append(tool.id)
                    }
                }
            }
        }
    }
}

/// The words the coding tools are introduced with, on the first run and in Settings alike.
enum Headline {
    static let tools = "All your coding agents on one channel."
}
