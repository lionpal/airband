import SwiftUI

/// The three things he chooses a model for. The same pickers are the body of each onboarding screen and of
/// each Settings page; which services and models exist, and what each needs, is the core's to say.
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
    let tap: () -> Void
    @ViewBuilder var extra: () -> Extra

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Button(action: tap) {
                HStack(spacing: 12) {
                    Image(systemName: chosen ? "checkmark.circle.fill" : "circle").font(.system(size: 22))
                        .foregroundStyle(chosen ? Palette.ink : Palette.dim)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(title).font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.ink)
                        if !subtitle.isEmpty { Text(subtitle).font(.system(size: 14)).foregroundStyle(Palette.soft) }
                    }
                    Spacer(minLength: 0)
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
    init(title: String, subtitle: String, chosen: Bool, tap: @escaping () -> Void) {
        self.init(title: title, subtitle: subtitle, chosen: chosen, open: false, tap: tap) { EmptyView() }
    }
}

/// What a service needs before it can be used: its key unless the phone already has one, and anything else it
/// is told. "Use it" tries it with a real call.
struct ServiceEntry: View {
    @EnvironmentObject var model: RadioModel
    let service: Service
    /// A new key if he typed one, and the settings he changed. Calls back with what is wrong, or nil.
    let use: (String?, [String: String], @escaping (String?) -> Void) -> Void
    @State private var key = ""
    @State private var told: [String: String] = [:]
    @State private var more = false
    @State private var checking = false
    @State private var wrong: String?

    private var kept: Bool { model.phoneKeys[service.kind] != nil }
    private var ready: Bool { !checking && !(service.key && !kept && key.trimmingCharacters(in: .whitespaces).isEmpty) }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if service.key {
                if kept {
                    Text("Uses the key you already gave").font(.system(size: 14)).foregroundStyle(Palette.soft)
                } else {
                    SecureField(service.keyName, text: $key).textContentType(.password).font(.system(size: 16))
                        .padding(.horizontal, 12).frame(minHeight: 44).background(Palette.field, in: RoundedRectangle(cornerRadius: 10))
                    if !service.keysAt.isEmpty { Text("Keys are made at \(service.keysAt)").font(.system(size: 12)).foregroundStyle(Palette.quiet) }
                }
            }
            if !service.asks.isEmpty {
                if more {
                    ForEach(service.asks, id: \.field) { ask in
                        TextField(ask.usual.isEmpty ? ask.question : "\(ask.question): \(ask.usual)", text: Binding(get: { told[ask.field] ?? "" }, set: { told[ask.field] = $0 }))
                            .textInputAutocapitalization(.never).autocorrectionDisabled().font(.system(size: 15))
                            .padding(.horizontal, 12).frame(minHeight: 40).background(Palette.field, in: RoundedRectangle(cornerRadius: 10))
                    }
                } else {
                    Button("More settings") { more = true }.font(.system(size: 14, weight: .medium)).foregroundStyle(Palette.quiet).buttonStyle(.plain)
                }
            }
            Button {
                (checking, wrong) = (true, nil)
                let fresh = key.trimmingCharacters(in: .whitespaces)
                use(fresh.isEmpty ? nil : fresh, told.filter { !$0.value.trimmingCharacters(in: .whitespaces).isEmpty }) { problem in
                    (checking, wrong) = (false, problem)
                    if problem == nil { key = "" }
                }
            } label: {
                HStack(spacing: 8) { Text("Use it"); if checking { ProgressView().controlSize(.small) } }
                    .font(.system(size: 16, weight: .semibold)).foregroundStyle(Palette.ground)
                    .frame(maxWidth: .infinity, minHeight: 44).background(ready ? Palette.ink : Palette.dim, in: RoundedRectangle(cornerRadius: 12))
            }
            .buttonStyle(.plain).disabled(!ready)
            if let wrong { Text(wrong).font(.system(size: 13)).foregroundStyle(.red) }
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

/// What reads the agents' messages and writes the short version. The work is done on his computer, so a
/// choice made here is sent there.
struct ReadsPicker: View {
    @EnvironmentObject var model: RadioModel
    let mode: ChoiceMode
    private let services = readingServices()
    @State private var open: String?
    @State private var asking: Service?
    @State private var busy: String?
    @State private var wrong: String?

    /// The one in use: what his computer says it reads with, or what he picked before a computer was paired.
    private var current: String? {
        if let waiting = model.pendingReads { return waiting }
        return services.first { $0.name == model.view.reads }?.kind
    }

    var body: some View {
        VStack(spacing: 10) {
            ForEach(services, id: \.kind) { s in
                let needs = s.key || !s.asks.isEmpty
                ChoiceCard(title: s.name, subtitle: s.key ? (model.phoneKeys[s.kind] != nil && current == s.kind ? "Using your \(s.keyName)" : "Enter your \(s.keyName)") : s.about, chosen: current == s.kind, open: open == s.kind) {
                    if current == s.kind { return }
                    if needs { open = open == s.kind ? nil : s.kind } else if mode == .settings { asking = s } else { apply(s, key: nil, settings: [:]) { _ in } }
                } extra: {
                    if busy == s.kind {
                        Fetching(label: "Asking your computer to switch", progress: nil)
                    } else if open == s.kind && current != s.kind {
                        ServiceEntry(service: s) { key, settings, done in apply(s, key: key, settings: settings, done: done) }
                    }
                }
            }
            if let wrong { Text(wrong).font(.system(size: 13)).foregroundStyle(.red).frame(maxWidth: .infinity, alignment: .leading) }
        }
        .confirmationDialog(asking.map { "Have \($0.name) write your summaries?" } ?? "", isPresented: Binding(get: { asking != nil }, set: { if !$0 { asking = nil } }), titleVisibility: .visible) {
            Button("Switch") { if let s = asking { apply(s, key: nil, settings: [:]) { _ in } } }
            Button("Cancel", role: .cancel) {}
        }
    }

    private func apply(_ s: Service, key: String?, settings: [String: String], done: @escaping (String?) -> Void) {
        (busy, wrong) = (s.kind, nil)
        model.chooseReads(s.kind, key: key, settings: settings) { problem in
            (busy, wrong) = (nil, problem)
            if problem == nil { open = nil }
            done(problem)
        }
    }
}

/// What turns his voice into words: a model on the phone, a service with his own key, or whatever his computer
/// is set up with.
struct EarsPicker: View {
    @EnvironmentObject var model: RadioModel
    let mode: ChoiceMode
    private let services = listeningServices()
    @State private var local: [LocalModel] = []
    @State private var open: String?
    @State private var asking: LocalModel?
    @State private var fetching: String?
    @State private var preparing = false
    @State private var progress = 0.0
    @State private var wrong: String?
    private let tick = Timer.publish(every: 0.25, on: .main, in: .common).autoconnect()

    var body: some View {
        VStack(spacing: 10) {
            ForEach(local, id: \.kind) { m in
                ChoiceCard(title: m.name, subtitle: m.fetched ? "Runs on this phone" : "Runs on this phone. \(m.megabytes) MB download", chosen: model.localEars == m.kind && model.ownEars.isEmpty, open: fetching == m.kind) {
                    guard model.localEars != m.kind || !model.ownEars.isEmpty else { return }
                    if mode == .settings { asking = m } else { model.forgetOwnEars(); model.useLocalEars(m.kind) }
                } extra: {
                    if fetching == m.kind {
                        Fetching(label: preparing ? "Getting it ready for this phone. About a minute, once." : "Downloading", progress: preparing ? nil : progress)
                    } else if model.localEars == m.kind, let last = model.lastHeard {
                        Text(String(format: "Last heard %.1f s of speech in %.2f s", last.of, last.took)).font(.system(size: 12)).foregroundStyle(Palette.quiet)
                    }
                    if !m.credit.isEmpty && model.localEars == m.kind { Text(m.credit).font(.system(size: 11)).foregroundStyle(Palette.quiet) }
                }
            }
            ForEach(services, id: \.kind) { s in
                ChoiceCard(title: s.name, subtitle: s.key ? (model.ownEars == s.kind ? "Using your \(s.keyName)" : "Enter your \(s.keyName)") : s.about, chosen: model.ownEars == s.kind, open: open == s.kind) {
                    if model.ownEars != s.kind { open = open == s.kind ? nil : s.kind }
                } extra: {
                    if open == s.kind && model.ownEars != s.kind {
                        ServiceEntry(service: s) { key, settings, done in
                            model.keepOwnEars(s.kind, key: key, settings: settings) { problem in
                                if problem == nil { open = nil; model.useLocalEars("") }
                                done(problem)
                            }
                        }
                    }
                }
            }
            ChoiceCard(title: "Your computer\u{2019}s choice", subtitle: "Whatever your computer is set up to listen with", chosen: model.localEars.isEmpty && model.ownEars.isEmpty) {
                model.forgetOwnEars()
                model.useLocalEars("")
            }
            if let wrong { Text(wrong).font(.system(size: 13)).foregroundStyle(.red).frame(maxWidth: .infinity, alignment: .leading) }
        }
        .onAppear { local = localListeners(dir: model.voiceDir) }
        .onReceive(tick) { _ in if fetching != nil && !preparing { progress = Double(localModelProgress()) / 100 } }
        .confirmationDialog(asking.map { $0.fetched ? "Listen with \($0.name) on this phone?" : "Download \($0.name) (\($0.megabytes) MB) and listen with it?" } ?? "", isPresented: Binding(get: { asking != nil }, set: { if !$0 { asking = nil } }), titleVisibility: .visible) {
            Button(asking?.fetched == true ? "Switch" : "Download") { if let m = asking { fetch(m) } }
            Button("Cancel", role: .cancel) {}
        }
    }

    /// Settings: fetch it if it is not here, fit it to this phone, then listen with it.
    private func fetch(_ m: LocalModel) {
        (fetching, preparing, progress, wrong) = (m.kind, m.fetched, 0, nil)
        let ready = {
            preparing = true
            model.prepareLocalEars(m.kind) { problem in
                (fetching, preparing, wrong) = (nil, false, problem)
                if problem == nil { model.forgetOwnEars(); model.useLocalEars(m.kind) }
                local = localListeners(dir: model.voiceDir)
            }
        }
        if m.fetched { return ready() }
        model.downloadVoice(m.kind) { problem in
            if let problem { (fetching, wrong) = (nil, problem) } else { ready() }
        }
    }
}

/// What speaks to him: a model on the phone (with a choice of voices), a service with his own key, or whatever
/// his computer is set up with.
struct VoicePicker: View {
    @EnvironmentObject var model: RadioModel
    let mode: ChoiceMode
    private let services = voiceServices()
    @State private var local: [LocalModel] = []
    @State private var open: String?
    @State private var asking: LocalModel?
    @State private var fetching: String?
    @State private var progress = 0.0
    @State private var wrong: String?
    private let tick = Timer.publish(every: 0.25, on: .main, in: .common).autoconnect()

    var body: some View {
        VStack(spacing: 10) {
            ForEach(local, id: \.kind) { m in
                ChoiceCard(title: m.name, subtitle: m.fetched ? "Runs on this phone" : "Runs on this phone. \(m.megabytes) MB download", chosen: model.localVoice == m.kind && model.serviceVoice.isEmpty, open: fetching == m.kind) {
                    guard model.localVoice != m.kind || !model.serviceVoice.isEmpty else { return }
                    if mode == .settings { asking = m } else { model.stopService(); model.useVoice(m.kind) }
                } extra: {
                    if fetching == m.kind {
                        Fetching(label: "Downloading", progress: progress)
                    } else {
                        VoiceTiles(kind: m.kind)
                    }
                }
            }
            ForEach(services, id: \.kind) { s in
                ChoiceCard(title: s.name, subtitle: s.key ? (model.serviceVoice == s.kind ? "Using your \(s.keyName)" : "Enter your \(s.keyName)") : s.about, chosen: model.serviceVoice == s.kind, open: open == s.kind) {
                    if model.serviceVoice != s.kind { open = open == s.kind ? nil : s.kind }
                } extra: {
                    if open == s.kind && model.serviceVoice != s.kind {
                        ServiceEntry(service: s) { key, settings, done in
                            model.useService(s.kind, key: key, settings: settings) { problem in
                                if problem == nil { open = nil }
                                done(problem)
                            }
                        }
                    } else if model.serviceVoice == s.kind {
                        Button("Hear it") { model.sampleVoice { wrong = $0 } }.font(.system(size: 15, weight: .semibold)).foregroundStyle(Palette.ink).buttonStyle(.plain)
                    }
                }
            }
            ChoiceCard(title: "Your computer\u{2019}s choice", subtitle: "Whatever your computer is set up to speak with", chosen: model.localVoice.isEmpty && model.serviceVoice.isEmpty) {
                model.stopService()
                model.useVoice("")
            }
            if let wrong { Text(wrong).font(.system(size: 13)).foregroundStyle(.red).frame(maxWidth: .infinity, alignment: .leading) }
        }
        .onAppear { local = localVoices(dir: model.voiceDir) }
        .onReceive(tick) { _ in if fetching != nil { progress = Double(localModelProgress()) / 100 } }
        .confirmationDialog(asking.map { $0.fetched ? "Speak with \($0.name) on this phone?" : "Download \($0.name) (\($0.megabytes) MB) and speak with it?" } ?? "", isPresented: Binding(get: { asking != nil }, set: { if !$0 { asking = nil } }), titleVisibility: .visible) {
            Button(asking?.fetched == true ? "Switch" : "Download") { if let m = asking { fetch(m) } }
            Button("Cancel", role: .cancel) {}
        }
    }

    private func fetch(_ m: LocalModel) {
        if m.fetched {
            model.stopService()
            return model.useVoice(m.kind)
        }
        (fetching, progress, wrong) = (m.kind, 0, nil)
        model.downloadVoice(m.kind) { problem in
            (fetching, wrong) = (nil, problem)
            if problem == nil { model.stopService(); model.useVoice(m.kind) }
            local = localVoices(dir: model.voiceDir)
        }
    }
}

/// The voices a voice model on the phone can speak in: a tile each, with a clip to hear first. The clips come
/// from where the model is kept, so they play before anything is downloaded.
struct VoiceTiles: View {
    @EnvironmentObject var model: RadioModel
    let kind: String

    var body: some View {
        let choices = localVoiceChoices(kind: kind)
        LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
            ForEach(choices, id: \.name) { choice in
                let chosen = model.voiceName == choice.name || (model.voiceName.isEmpty && choice.name == choices.first?.name)
                HStack(spacing: 9) {
                    Button { model.preview(choice) } label: {
                        Image(systemName: model.previewing == choice.name ? "stop.circle.fill" : "play.circle.fill").font(.system(size: 28))
                            .foregroundStyle(chosen ? Palette.ground : Palette.ink)
                    }
                    .buttonStyle(.plain).accessibilityLabel(model.previewing == choice.name ? "Stop \(choice.name)" : "Hear \(choice.name)")
                    Button { model.chooseVoice(choice.name) } label: {
                        VStack(alignment: .leading, spacing: 0) {
                            Text(choice.name).font(.system(size: 15, weight: .semibold))
                            Text(choice.about).font(.system(size: 12)).lineLimit(1).minimumScaleFactor(0.8)
                        }
                        .foregroundStyle(chosen ? Palette.ground : Palette.ink).frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain).accessibilityAddTraits(chosen ? .isSelected : [])
                }
                .padding(.horizontal, 10).padding(.vertical, 9)
                .background(chosen ? Palette.ink : Palette.raised, in: RoundedRectangle(cornerRadius: 12))
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
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text(headline).font(.system(size: 26, weight: .bold)).tracking(-0.5).foregroundStyle(Palette.ink).fixedSize(horizontal: false, vertical: true)
                picker()
            }
            .padding(.horizontal, 20).padding(.vertical, 16).frame(maxWidth: 560).frame(maxWidth: .infinity)
        }
        .background(Palette.ground.ignoresSafeArea())
        .navigationTitle(title).navigationBarTitleDisplayMode(.inline)
        .preferredColorScheme(.dark)
    }
}

/// The words each of the three is introduced with, in onboarding and in Settings alike.
enum Headline {
    static let reads = "Long messages, cut down to what you need to read."
    static let ears = "Talk to your agents."
    static let voice = "Your agents talk back."
}
