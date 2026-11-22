// Glue between the Rust core and the screen. The core's desk holds every paired computer as one and decides
// everything about the chat: what is waiting, what is done, where a message goes, who has the floor on a call.
// This file passes the desk's view to SwiftUI, lends it the phone's speaker and microphone, and keeps the
// pairing links in the Keychain. Nothing here would have to be written again in the same way for Android or
// Windows beyond these three things.
import AVFoundation
import PhononCoreML
import Foundation
import ActivityKit
import SwiftUI
import UIKit

final class RadioModel: ObservableObject, DeskHost, @unchecked Sendable {
    static let shared = RadioModel()

    /// What the screen shows, as the desk last gave it, for the room or agent he is looking at.
    @Published var view = DeskView(paired: false, online: false, connecting: false, near: false, away: [], feed: [], rooms: [], unread: 0, canPoke: false, quick: [], sending: [], stations: [], mainComputer: nil, chooseMain: false, reads: "", uploads: [], asking: nil)
    /// A project: show just its agents, and keep what he says among them.
    @Published var room: String? { didSet { refresh() } }
    /// An agent, by id: show just that agent.
    @Published var only: String? { didSet { refresh() } }
    /// What the agent he is looking at is called now.
    var onlyName: String? { only.flatMap { id in agents.first { $0.id == id }?.callsign } }
    @Published var full: [String: String] = [:] // message id → exactly what the agent wrote
    /// He reads everything each agent wrote, not the short form, unless he closes one.
    @Published var showWhole = UserDefaults.standard.bool(forKey: "airband.show.whole") { didSet { UserDefaults.standard.set(showWhole, forKey: "airband.show.whole") } }
    @Published var pictureFiles: [String: String] = [:] // picture id → the file it was fetched into
    @Published var replyingTo: ChatMessage?
    /// Something he must be told. It goes by itself after a few seconds; a tap takes it away sooner.
    @Published var problem: String? {
        didSet {
            guard let shown = problem else { return }
            DispatchQueue.main.asyncAfter(deadline: .now() + 6) { [weak self] in if self?.problem == shown { self?.problem = nil } }
        }
    }
    // call mode
    @Published var calling = false
    @Published var status: RadioStatus = .idle
    @Published var hearing = "" // what he has said so far, while he is still speaking
    /// What the voice is saying now, when the voice service says when each word is spoken: the text, where each
    /// word starts in it, when each is said (seconds into the clip), and when the clip began to sound.
    @Published var caption: Caption?
    /// A question's keyboard button was tapped: the screen opens the text box for it.
    @Published var typeNow = false
    @Published var micOpen = false // he opened the microphone and has not sent or taken back what he said
    private var dictating = false // from his tap until the core has dealt with what he said
    /// The microphone is open: his own tap opened it, or on a call it opened by itself for a question.
    var listening: Bool {
        if micOpen { return true }
        if case .listening = status { return !dictating }
        return false
    }
    @Published var reading: String? // the message being read aloud from its full text, by id
    @Published var tray: [TrayItem] = [] // photos and videos he has attached and not yet sent
    @Published var accounts: [StationAccounts] = [] // each computer's accounts, as last read
    @Published var accountsLoading = false
    @Published var switching: String? // the account being switched to, as "station|id"
    @Published var accountsProblem: String?

    let voice = Voice()
    private var desk: Desk!
    private var links: [String] = []
    private var fetching: Set<String> = []
    /// Fetching full text and pictures: a few at a time on shared threads, not a thread each.
    private let fetch = DispatchQueue(label: "dev.airband.fetch", qos: .userInitiated, attributes: .concurrent)
    /// Whether the app is in front of him, readable from any thread.
    private let frontLock = NSLock()
    private var front = true

    var paired: Bool { view.paired }
    var online: Bool { view.online }
    var connecting: Bool { view.connecting }

    init() {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("radio")
        voiceDir = dir.path
        // the newest catalogue of services and models this phone has kept, before anything is listed from it
        loadCatalog(dir: dir.path)
        desk = Desk.open(dataDir: dir.path, host: self)
        #if DEBUG
        // development only: `-airband.devTools '("codex")'` opens as someone who uses those coding tools, for a screenshot
        if let tools = UserDefaults.standard.stringArray(forKey: "airband.devTools") { desk.setTools(tools: tools) }
        #endif
        var links = (try? JSONDecoder().decode([String].self, from: Data((Keychain.read("airband.links") ?? "[]").utf8))) ?? []
        #if DEBUG
        // development only: `xcrun simctl launch <sim> dev.airband.app -airband.devLink <pairing link>` pairs without the
        // system's "Open in airband?" prompt, which a script cannot tap
        if let dev = UserDefaults.standard.string(forKey: "airband.devLink"), isPairingLink(link: dev.split(separator: " ").first.map(String.init) ?? "") {
            links = dev.split(separator: " ").map(String.init) // several links, separated by spaces
        }
        #endif
        #if DEBUG
        // development only: `-airband.devMic YES` opens the microphone a few seconds after launch, as a tap on
        // the big button would, for a run nobody is there to tap
        // the models he chose are fetched first, the microphone is open for eight seconds, and what it heard is sent
        if UserDefaults.standard.bool(forKey: "airband.devMic") {
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
                self?.finishChoices { problem in
                    NSLog("airband dev: models fetched: \(problem ?? "ok")")
                    Task { await self?.talk() }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 8) { Task { NSLog("airband dev: closing the microphone"); await self?.talk() } }
                }
            }
        }
        #endif
        let center = NotificationCenter.default
        center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
            self?.setFront(true)
            // a request left open when the app went away is dead by now: ask the computers again, at once
            DispatchQueue.global(qos: .userInitiated).async { self?.desk.wake() }
        }
        center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { [weak self] _ in self?.wentAway() }
        center.addObserver(forName: UIApplication.willResignActiveNotification, object: nil, queue: .main) { [weak self] _ in self?.setFront(false) }
        connect(links)
    }

    /// The app was put away. Reading messages aloud goes on only where that is wanted with the screen off:
    /// through headphones or a car. From the phone's own speaker it stops.
    private func wentAway() {
        guard calling else { return }
        let onPhone: [AVAudioSession.Port] = [.builtInSpeaker, .builtInReceiver]
        let handsFree = AVAudioSession.sharedInstance().currentRoute.outputs.contains(where: { !onPhone.contains($0.portType) })
        if !handsFree { Task { await setCalling(false) } }
    }

    private func setFront(_ on: Bool) {
        frontLock.lock()
        front = on
        frontLock.unlock()
    }

    /// The link `airband pair` prints: opened from Messages or Mail, or pasted. Each computer has its own; pairing
    /// another adds it.
    func pair(with link: String) {
        let link = link.trimmingCharacters(in: .whitespacesAndNewlines)
        guard isPairingLink(link: link) else {
            problem = "That is not a pairing link. Run airband pair on your computer."
            return
        }
        guard !links.contains(link) else { return }
        connect(links + [link])
    }

    func forgetAll() { connect([]) }

    /// Where this phone can be woken, as Apple gave it: every paired computer is told.
    func pushAddress(_ token: String) { desk.setPush(provider: .apple, token: token) }

    /// An answer given on a notification. Blocks until the computer has it, or says why it does not.
    func answerNow(station: String, message: String, text: String, tapped: Tapped?) throws { try desk.answerNow(station: station, messageId: message, text: text, tapped: tapped) }

    /// Look at everyone, at one team, or at one agent. Whatever he was answering is let go: it may no longer be
    /// in view. Every place that changes who he is looking at goes through here.
    func look(room: String? = nil, agent: String? = nil) {
        replyingTo = nil
        self.room = room
        only = agent
    }

    /// He tapped a notification: show that agent.
    func show(room: String, agent: String) {
        look(room: room.isEmpty ? nil : room, agent: agent.isEmpty ? nil : agent)
    }

    /// Every agent on the radio, in the order the teams are shown.
    var agents: [DeskMember] { view.rooms.flatMap(\.members) }
    /// His computers that gave themselves a name.
    var computers: [String] { view.stations.filter { !$0.isEmpty } }
    /// What he is told when a computer cannot be reached: which, when more than one could be meant.
    var cannotReach: String { view.away.isEmpty ? "Can\u{2019}t reach your computer" : "Can\u{2019}t reach " + view.away.joined(separator: ", ") }
    /// A state as it is read out for someone who cannot see its dot.
    static func spoken(state: AgentState) -> String {
        switch state {
        case .asking: "waiting on you"
        case .stalled: "stopped"
        case .working: "working"
        case .idle: "idle"
        case .unknown: "state not known"
        }
    }

    /// A message's words whole, once they have been fetched; as it came until then.
    func whole(_ m: ChatMessage) -> String { full[m.id] ?? m.text }

    private func connect(_ wanted: [String]) {
        links = desk.setLinks(links: wanted)
        if links.count < wanted.count { problem = "One pairing link is not valid and was skipped." }
        Keychain.write("airband.links", String(data: (try? JSONEncoder().encode(links)) ?? Data("[]".utf8), encoding: .utf8) ?? "[]")
        refresh()
        // paired: this is when notifications are worth asking for, and when a new computer needs the phone's address
        if !links.isEmpty { DispatchQueue.main.async { Push.shared.setUp() } }
    }

    private func refresh() {
        // only when something he can see has changed: the chat is not laid out again for nothing
        let now = desk.view(room: room, agent: only)
        if now != view { view = now }
        // what has gone with a message has left the desk's list: it leaves the tray too
        if !tray.isEmpty {
            let gone = tray.filter { item in item.attachment.map { id in !view.uploads.contains { $0.id == id } } ?? false }
            gone.forEach { if let source = $0.source { try? FileManager.default.removeItem(at: source) } }
            tray.removeAll { item in gone.contains { $0.id == item.id } }
        }
        // a room that has gone (its last agent left) is not somewhere he can stay
        if let room, view.online, !view.rooms.contains(where: { $0.name == room }) { self.room = nil }
        if full.count > (showWhole ? 400 : 60) { full.removeAll() } // opened long ago: fetched again if he looks again
        refreshChoices()
        syncBanners()
    }

    /// An address the app was opened with: a pairing link, or what the Talk tile and a banner open it with.
    func open(_ url: URL) {
        switch OpenWith.route(url) {
        case .talk:
            replyingTo = nil
            Task { @MainActor in if !self.micOpen { await self.talk() } }
        case .answer(let agent?):
            answer(agent)
        case .answer(nil):
            Task { @MainActor in if !self.micOpen { await self.talk() } }
        case nil:
            pair(with: url.absoluteString)
        }
    }

    /// Answer an agent that is waiting on him, hands free: its question is read out again, with the answers
    /// it offers, and then the microphone opens. What he says is the answer to that question, in whatever
    /// words he uses.
    func answer(_ id: String) {
        guard let team = view.rooms.first(where: { $0.members.contains { $0.id == id } }), let agent = team.members.first(where: { $0.id == id }) else { return }
        // look at that agent, so the question is in view and his words go to it
        replyingTo = nil
        room = team.name
        only = id
        let question = view.feed.last { $0.id == agent.questionId }
        guard !agent.questionId.isEmpty else {
            Task { @MainActor in if !self.micOpen { await self.talk() } }
            return
        }
        stopReading()
        reading = agent.questionId
        DispatchQueue.global(qos: .userInitiated).async {
            _ = self.desk.readQuestion(messageId: agent.questionId)
            DispatchQueue.main.async {
                // he stopped the reading, or went elsewhere: the microphone stays shut
                guard self.reading == agent.questionId else { return }
                self.reading = nil
                Task { @MainActor in if !self.micOpen { await self.talk(about: question) } }
            }
        }
    }

    /// The banners up now, by the agent each is for.
    private var banners: [String: Activity<NeedsYouAttributes>] = [:]

    /// While he is on a call, each agent that is waiting on him has a banner: on the lock screen, and on a
    /// car's screen when another app is in front. It comes down when the agent has its answer or the call ends.
    private func syncBanners() {
        let waiting = calling ? agents.filter { $0.state == .asking } : []
        for (id, banner) in banners where !waiting.contains(where: { $0.id == id }) {
            banners[id] = nil
            Task { await banner.end(nil, dismissalPolicy: .immediate) }
        }
        guard !waiting.isEmpty, ActivityAuthorizationInfo().areActivitiesEnabled else { return }
        for agent in waiting where banners[agent.id] == nil {
            let state = NeedsYouAttributes.ContentState(waiting: waiting.count, question: agent.question)
            guard let banner = try? Activity.request(attributes: NeedsYouAttributes(agent: agent.id, callsign: agent.callsign, badge: agent.badge), content: .init(state: state, staleDate: nil)) else { continue }
            banners[agent.id] = banner
            // the alert is what makes it show itself, and not only sit on the lock screen
            Task { await banner.update(.init(state: state, staleDate: nil), alertConfiguration: .init(title: "\(agent.callsign) needs you", body: LocalizedStringResource(stringLiteral: agent.question.isEmpty ? "Tap to answer" : agent.question), sound: .default)) }
        }
    }

    /// The coding tools he uses, by id, the one he uses most first. The core keeps them: what can write his
    /// summaries follows from them.
    var tools: [String] {
        get { desk.tools() }
        set {
            objectWillChange.send()
            desk.setTools(tools: newValue)
            refreshChoices()
        }
    }

    /// The picture he chose for each of his computers, by its name: a system symbol's name.
    @Published var computerIcons: [String: String] = (try? JSONDecoder().decode([String: String].self, from: Data((UserDefaults.standard.string(forKey: "airband.computers.icons") ?? "{}").utf8))) ?? [:]
    /// The pictures a computer can have.
    static let computerSymbols = ["laptopcomputer", "macmini", "desktopcomputer", "macstudio", "macpro.gen3", "server.rack", "pc"]

    func icon(of computer: String) -> String { computerIcons[computer] ?? "desktopcomputer" }
    /// What the main computer is for, in the words used wherever he chooses it.
    static let mainComputerNote = "When it isn\u{2019}t clear which computer your words are for, they go to your main computer. It works out who they are for and passes them on."
    func setMainComputer(_ computer: String) {
        guard !computer.isEmpty else { return }
        desk.setMainComputer(computer: computer)
    }
    func setIcon(_ symbol: String, of computer: String) {
        computerIcons[computer] = symbol
        UserDefaults.standard.set(String(data: (try? JSONEncoder().encode(computerIcons)) ?? Data("{}".utf8), encoding: .utf8), forKey: "airband.computers.icons")
    }

    /// The folders inside one of his on a computer. Calls back with them, or with what is wrong.
    func folders(on computer: String, under: String, done: @escaping (DeskFolders?, String?) -> Void) {
        ask(Self.unreached, { try self.desk.folders(computer: computer, under: under) }, then: done)
    }

    /// The folders the coding tools on a computer have had sessions in, newest first. Calls back with them, or
    /// with what is wrong.
    func recentFolders(on computer: String, done: @escaping ([DeskRecent]?, String?) -> Void) {
        ask(Self.unreached, { try self.desk.recentFolders(computer: computer) }, then: done)
    }

    /// Start an agent in a folder on a computer: in the team that works there, or a new one of this name.
    /// Calls back with the team's name as the computer writes it, or with what is wrong.
    func newTeam(_ name: String, on computer: String, folder: String, make: Bool, tool: String?, done: @escaping (String?, String?) -> Void) {
        ask(Self.unreached, { try self.desk.newTeam(computer: computer, name: name, folder: folder, make: make, tool: tool) }, then: done)
    }

    /// Goes up when a newer catalogue of services and models has been taken: what the choosers list is read again.
    @Published var catalogRev = 0
    private var catalogAsked = Date.distantPast

    /// Ask for a newer catalogue, at most once every few minutes. The choosers call this when they open.
    func refreshCatalog() {
        guard Date().timeIntervalSince(catalogAsked) > 300 else { return }
        catalogAsked = Date()
        let dir = voiceDir
        fetch.async {
            if Airband.refreshCatalog(dir: dir) { DispatchQueue.main.async { self.catalogRev += 1 } }
        }
    }

    /// What he can choose for each job (what writes his summaries, what listens, what speaks) and how each
    /// choice stands, as the core last said. The core decides all of it; the screens draw this.
    @Published var choices: [ChoiceJob: [Choice]] = [:]

    /// Ask the core again. Cheap: nothing is asked of a network.
    func refreshChoices() {
        let now = Dictionary(uniqueKeysWithValues: desk.choiceJobs().map { ($0, desk.choices(job: $0)) })
        if now != choices { choices = now }
    }

    /// The jobs he chooses something for, in the order the first run asks about them.
    var jobs: [ChoiceJob] { desk.choiceJobs() }

    /// What is chosen for a job, in a few words.
    func chosenLabel(_ job: ChoiceJob) -> String { desk.chosenLabel(job: job) }

    /// Choose something for a job. `key` is a key he has just typed; `later` only notes a model, to be fetched
    /// by `finishChoices`. The core tries a service, or fetches a model, before it takes it. Calls back with
    /// what is wrong, or nil.
    func choose(_ job: ChoiceJob, _ id: String, key: String? = nil, settings: [String: String] = [:], later: Bool = false, done: @escaping (String?) -> Void = { _ in }) {
        work(done) { try self.desk.chooseFor(job: job, id: id, key: key, settings: settings, later: later) }
    }

    /// Speak in this one of the chosen voice's voices.
    func chooseVoice(_ name: String, done: @escaping (String?) -> Void = { _ in }) {
        work(done) { try self.desk.chooseVoice(name: name) }
    }

    /// The end of the first run: fetch the models he chose that are not on this phone yet.
    func finishChoices(done: @escaping (String?) -> Void) {
        work(done) { try self.desk.finishChoices() }
    }

    /// What the core is doing about a choice right now, and how far along it is.
    func choiceWork() -> (label: String, fraction: Double?)? {
        if case .working(let label, let fraction)? = desk.choiceWork() { return (label, fraction) }
        return nil
    }

    /// What he is told when a computer could not be asked.
    static let unreached = "Cannot reach that computer."

    /// What went wrong, in words for him: the reason the computer gave when it gave one, that it cannot be
    /// reached when it cannot, and otherwise `fallback`, which says what did not happen.
    static func words(for error: Error, else fallback: String) -> String {
        switch error {
        case RadioError.Told(let reason): return reason
        case RadioError.Offline: return unreached
        default: return fallback
        }
    }

    /// Run one of the core's blocking calls off the main thread and come back on it: with what the call gave,
    /// or with what went wrong in words. Every call that can fail goes through here.
    private func ask<T>(_ fallback: String, _ call: @escaping () throws -> T, then: @escaping (T?, String?) -> Void) {
        fetch.async {
            let result = Result { try call() }
            DispatchQueue.main.async {
                switch result {
                case .success(let got): then(got, nil)
                case .failure(let error): then(nil, Self.words(for: error, else: fallback))
                }
            }
        }
    }

    /// The same, for something done to an agent or a message: what went wrong is shown to him, and `done`
    /// runs when nothing did.
    private func tell(_ fallback: String, _ call: @escaping () throws -> Void, done: @escaping () -> Void = {}) {
        ask(fallback, call) { _, wrong in
            if let wrong { self.problem = wrong } else { done() }
        }
    }

    /// One of the core's calls that changes what is chosen. Calls back with what it said was wrong.
    private func work(_ done: @escaping (String?) -> Void, _ call: @escaping () throws -> Void) {
        ask("Your computer could not be reached. Try again when it can.", call) { _, wrong in
            // a model can be fetched again, so it is left out of the phone's backups
            var folder = URL(fileURLWithPath: self.voiceDir).appendingPathComponent("models")
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try? folder.setResourceValues(values)
            self.refreshChoices()
            done(wrong)
        }
    }

    /// One tap on an answer button: it goes as what the button stood for.
    func tap(_ m: ChatMessage, _ tapped: Tapped) {
        if replyingTo?.id == m.id { replyingTo = nil }
        desk.tap(messageId: m.id, tapped: tapped)
    }
    /// Words sent as the answer to that message.
    func answer(_ m: ChatMessage, _ words: String) {
        if replyingTo?.id == m.id { replyingTo = nil }
        desk.answer(messageId: m.id, text: words)
    }

    /// Send what he typed: to the message he picked if he picked one, else to whoever the desk and the tower
    /// work out from his words and where he is looking.
    func send(_ text: String) {
        let target = replyingTo
        replyingTo = nil
        // with nothing typed there is still something to send when files are attached: the core sends the files alone
        desk.send(text: text, re: target?.id, room: room, agent: only)
    }

    /// Try the computers again, now.
    func reconnect() {
        DispatchQueue.global(qos: .userInitiated).async { self.desk.reconnect() }
    }

    /// The agents he could stop right now: at work, run by their computer, and in the room or agent he is looking at.
    var stoppable: [DeskMember] {
        view.rooms.filter { room == nil || $0.name == room }.flatMap(\.members).filter { $0.held && $0.state == .working && (only == nil || $0.id == only) }
    }

    /// Stop what an agent is doing, now. What he said to it meanwhile it then reads together.
    func stop(_ agent: DeskMember) {
        tell("It did not stop.") { try self.desk.stopAgent(agent: agent.id) }
    }

    /// End an agent its computer runs: it finishes and leaves the radio.
    func end(_ agent: DeskMember) {
        tell("It did not end.", { try self.desk.endAgent(agent: agent.id) }) { if self.only == agent.id { self.only = nil } }
    }

    /// The app's own folder: the core keeps what he chose there, and the models that run on this phone.
    let voiceDir: String

    /// The keys he has given this phone, by provider. Kept in the Keychain, on this phone only. Only the core
    /// asks for one, to use it: no screen is ever handed a key.
    private var keys: [String: String] {
        (try? JSONDecoder().decode([String: String].self, from: Data((Keychain.read("airband.keys") ?? "{}").utf8))) ?? [:]
    }
    private let keysLock = NSLock()
    func secret(name: String) -> String? {
        keysLock.lock()
        defer { keysLock.unlock() }
        return keys[name]
    }
    func keepSecret(name: String, value: String?) {
        keysLock.lock()
        defer { keysLock.unlock() }
        var all = keys
        all[name] = value
        Keychain.write("airband.keys", String(data: (try? JSONEncoder().encode(all)) ?? Data("{}".utf8), encoding: .utf8) ?? "{}")
    }

    /// The clip being played to let him hear a voice before choosing it, and which voice it is.
    @Published var previewing: String?
    private var previewPlayer: AVAudioPlayer?

    /// Play a short clip of a voice. It is fetched from where the model is kept, so it works before the model
    /// is on the phone. A second tap on the same one stops it.
    func preview(_ choice: VoiceChoice) {
        if previewing == choice.name {
            previewPlayer?.stop()
            previewing = nil
            return
        }
        previewing = choice.name
        guard let url = URL(string: choice.preview) else { return }
        URLSession.shared.dataTask(with: url) { data, _, _ in
            DispatchQueue.main.async {
                guard self.previewing == choice.name, let data, let player = try? AVAudioPlayer(data: data) else {
                    if self.previewing == choice.name { self.previewing = nil }
                    return
                }
                try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .default, options: [.mixWithOthers])
                try? AVAudioSession.sharedInstance().setActive(true)
                self.previewPlayer = player
                player.play()
                // the row stops showing it as playing when the clip is over
                DispatchQueue.main.asyncAfter(deadline: .now() + player.duration + 0.2) { if self.previewing == choice.name { self.previewing = nil } }
            }
        }.resume()
    }

    /// Say a sentence in the chosen voice, so he hears what he picked.
    func sampleVoice(done: @escaping (String?) -> Void) {
        ask("The voice did not speak.", {
            let sample = try self.desk.ownVoiceSample()
            self.voice.play(rate: sample.rate, pcm: sample.pcm, last: true)
        }) { _, wrong in done(wrong) }
    }

    /// The engines this app runs models with itself, by the name a model's row in the catalogue gives. The
    /// core offers a model for one of these only on a system its row allows, and asks for it by this name.
    private lazy var engines: [String: (String, ChoiceJob, Data) -> Data?] = ["phonon-coreml": { [unowned self] in self.phononCoreML(folder: $0, job: $1, input: $2) }]

    func device() -> HostDevice {
        HostDevice(engines: engines.keys.sorted(), os: "ios", osVersion: UInt32(ProcessInfo.processInfo.operatingSystemVersion.majorVersion))
    }

    /// The core asks for a model to be run by one of the engines above.
    func runModel(engine: String, folder: String, job: ChoiceJob, input: Data) -> Data? {
        engines[engine]?(folder, job, input)
    }

    /// The models loaded so far, by the folder each is in. Loading one takes a moment, so it is kept.
    private var heardWith: [String: Transcriber] = [:]
    private let hearLock = NSLock()

    /// Fermion's listening model on the Neural Engine: a recording (16 kHz, 16-bit, mono) turned into words.
    private func phononCoreML(folder: String, job: ChoiceJob, input pcm: Data) -> Data? {
        guard job == .ears else { return nil }
        hearLock.lock()
        defer { hearLock.unlock() }
        do {
            let model = try heardWith[folder] ?? Transcriber(bundle: URL(fileURLWithPath: folder))
            heardWith[folder] = model
            let samples: [Float] = pcm.withUnsafeBytes { raw in raw.bindMemory(to: Int16.self).map { Float($0) / 32768 } }
            return Data(try model.transcribe(samples).text.utf8)
        } catch {
            NSLog("airband: the listening model failed: \(error.localizedDescription)")
            return nil
        }
    }

    /// The computers a new agent of this team can be started on, the usual one first.
    func computers(for room: String) -> [String] { desk.computersFor(room: room) }

    /// Start a new agent in a team. It appears in the list at once, with nothing to do until he speaks to it.
    /// `tool` is one of the coding tools he uses; nothing is the one he uses when there is only one.
    func newAgent(in room: String, on computer: String?, tool: CodingTool? = nil) {
        desk.newAgent(room: room, computer: computer, tool: tool ?? (usedTools.count == 1 ? usedTools.first : nil))
    }

    /// The coding tools he uses, as the catalogue describes them, the one he uses most first.
    var usedTools: [CodingTool] {
        let all = codingTools()
        return tools.compactMap { id in all.first { $0.id == id } }
    }

    /// Let a question go without answering it.
    func dismiss(_ m: ChatMessage) {
        if replyingTo?.id == m.id { replyingTo = nil }
        tell("It was not let go.") { try self.desk.dismiss(messageId: m.id) }
    }

    /// Try again a message that was not sent.
    func sendAgain(_ o: Outgoing) { desk.sendAgain(id: o.id) }
    /// Let go of a message that was not sent.
    func discardUnsent(_ o: Outgoing) { _ = desk.discardUnsent(id: o.id) }
    /// How each computer was last reached, as this phone saw it.
    func reach() -> [DeskReach] { desk.reach() }
    /// The record of failed requests, for him to copy. Made here and sent nowhere.
    func diagnostics() -> String { desk.diagnostics() }

    /// Take back something he said that is still waiting its turn.
    func unsend(_ m: ChatMessage) {
        tell("It was not taken back.") { try self.desk.unsend(messageId: m.id) }
    }

    /// Have its computer take over an agent he started in a terminal, so his words reach it as his own.
    func takeOver(_ agent: DeskMember) {
        tell("It was not taken over.") { try self.desk.takeOver(agent: agent.id) }
    }

    /// Send the words of one of his messages again, to another agent.
    func resend(_ m: ChatMessage, to agent: DeskMember) {
        fetch.async { self.desk.resend(messageId: m.id, agentId: agent.id) }
    }

    /// The phone has made a picked photo or video ready (or could not): hand it to the desk, which sends it to
    /// whoever he is looking at, and remember the id it travels under.
    @MainActor func prepared(_ item: UUID, _ made: (URL, String, UIImage?)?, source: URL?) {
        guard let i = tray.firstIndex(where: { $0.id == item }) else {
            // he took it off while it was being made
            if let made { try? FileManager.default.removeItem(at: made.0) }
            if let source { try? FileManager.default.removeItem(at: source) }
            return
        }
        tray[i].source = source
        guard let made else {
            tray[i].failed = true
            return
        }
        tray[i].thumb = made.2
        tray[i].attachment = desk.attach(filePath: made.0.path, name: made.1, re: replyingTo?.id, room: room, agent: only)
    }

    /// Stop sending something that is on its way, without taking it out of the tray.
    func dropUpload(_ id: String) { desk.detach(id: id) }

    /// Take one off again before it is sent.
    func detach(_ item: TrayItem) {
        if let id = item.attachment { desk.detach(id: id) }
        if let source = item.source { try? FileManager.default.removeItem(at: source) }
        tray.removeAll { $0.id == item.id }
    }

    /// One tap on a quick message: the feed shows its label and the agent gets the whole instruction. It goes to
    /// the message he picked, or to whoever the tower works out.
    func quick(_ q: QuickMessage) {
        let target = replyingTo
        replyingTo = nil
        desk.quick(label: q.label, text: q.text, re: target?.id, room: room, agent: only)
    }

    /// Set going again everyone who has stopped without finishing: in the team he is looking at, or everywhere.
    func poke() { desk.poke(room: room) }

    /// What was said between him and this agent just before a message.
    func thread(before m: ChatMessage) -> [ChatMessage] { desk.thread(messageId: m.id) }

    /// Fetch what the agent actually wrote, once.
    func loadFull(_ m: ChatMessage) {
        guard m.long, full[m.id] == nil else { return }
        full[m.id] = m.text // shown until the real text arrives, and stops a second request
        fetch.async {
            guard let text = try? self.desk.full(messageId: m.id) else { return }
            DispatchQueue.main.async { self.full[m.id] = text }
        }
    }

    func stopReading() {
        guard reading != nil else { return }
        reading = nil
        desk.stopReading()
        voice.hush()
    }

    /// Read every computer's accounts. `fresh` asks each tower to check the allowances again now.
    func loadAccounts(fresh: Bool) {
        guard paired, !accountsLoading else { return }
        accountsLoading = true
        fetch.async {
            let got = self.desk.accounts(fresh: fresh)
            DispatchQueue.main.async {
                self.accounts = got
                self.accountsLoading = false
            }
        }
    }

    /// Move one computer's sessions to another of its accounts. Everything about how is the tower's business.
    func useAccount(_ station: String, _ a: AccountEntry) {
        guard switching == nil, !a.live else { return }
        switching = station + "|" + a.id
        accountsProblem = nil
        ask("It did not switch.", { try self.desk.useAccount(station: station, harness: a.harness, id: a.id) }) { now, wrong in
            self.switching = nil
            self.accountsProblem = wrong
            if let now, let i = self.accounts.firstIndex(where: { $0.station == station }) { self.accounts[i] = now }
        }
    }

    /// Put an agent in another team: one that is there, or a new one of that name. If he was looking at that
    /// agent alone, he still is, under its new name.
    func renameTeam(_ team: String, to name: String) {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, name != team else { return }
        ask("It was not renamed.", { try self.desk.renameTeam(team: team, to: name) }) { now, wrong in
            if let wrong { self.problem = wrong; return }
            // still looking at the same team, under its new name
            if self.room == team, let now { self.room = now }
        }
    }

    func moveAgent(_ agent: DeskMember, to team: String) {
        let team = team.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !team.isEmpty else { return }
        tell("It did not move.", { try self.desk.moveAgent(agent: agent.id, team: team) }) {}
    }

    /// Fetch a picture, once. The core keeps the file, so after the first time this is immediate and works offline.
    func loadPicture(_ m: ChatMessage, _ p: ChatPicture) {
        guard pictureFiles[p.id] == nil, !fetching.contains(p.id) else { return }
        fetching.insert(p.id)
        fetch.async {
            let path = try? self.desk.picture(messageId: m.id, pictureId: p.id)
            DispatchQueue.main.async {
                self.fetching.remove(p.id) // a failed fetch is tried again the next time it is on screen
                if let path { self.pictureFiles[p.id] = path }
            }
        }
    }

    @MainActor func setCalling(_ on: Bool) async {
        if on {
            if let missing = await voice.authorize() {
                problem = missing
                return
            }
            voice.goOnDuty()
        }
        calling = on
        // off: the core is told first, so it hands over no more sound, and then what is playing is cut
        desk.setCallMode(on: on)
        if !on { voice.goOffDuty() }
    }

    /// The big button: he wants to say something now. It is the answer to the message he picked, if he picked one;
    /// otherwise the tower works out who it is for, in the room he is looking at or among everyone.
    /// The microphone stays open until he taps again, however long he pauses: that second tap sends what he
    /// said. Nothing is ever sent because he stopped talking.
    @MainActor func talk(about message: ChatMessage? = nil) async {
        // the microphone is open: this tap closes it, and what he said goes
        if listening {
            micOpen = false
            voice.closeMic()
            return
        }
        if let missing = await voice.authorize() {
            problem = missing
            return
        }
        // nothing is chosen to listen with, or what he chose cannot: he is told, and the microphone stays shut
        if let why = desk.cannotListen() {
            note(text: why)
            return
        }
        // open at once, on its own queue: the screen and the microphone do not wait for anything else
        micOpen = true
        dictating = true
        hearing = ""
        voice.openMic()
        stopReading()
        // `message`: he is looking at something of one agent's (a picture, say) and this is about that
        let target = message ?? replyingTo
        replyingTo = nil
        desk.talk(re: target?.id, room: only == nil ? room : nil, agent: target == nil ? only : nil)
    }

    /// He took back what he was saying: the microphone closes and nothing is sent. Returns what had been heard,
    /// for when he wants to finish it by hand.
    @discardableResult func cancelTalk() -> String {
        let heard = hearing
        micOpen = false
        desk.cancelTalk()
        voice.closeMic()
        hearing = ""
        return heard
    }

    /// His words could not be placed and he has said who they are for, or let them go.
    func choose(_ choice: DeskChoice?) {
        if let choice { desk.choose(agent: choice.id) } else { desk.dropAsk() }
    }

    // MARK: DeskHost: called by the core, on its own threads

    func play(rate: UInt32, pcm: Data, last: Bool) { voice.play(rate: rate, pcm: pcm, last: last) }
    func recordStart(rate: UInt32) -> Bool { voice.recordStart(rate: rate) }
    func recordRead() -> Data { voice.recordRead() }
    func recordStop() { voice.recordStop() }
    func changed() { DispatchQueue.main.async { self.refresh() } }
    func status(status: RadioStatus) {
        DispatchQueue.main.async {
            self.status = status
            if case .speaking = status {} else { self.caption = nil }
            // the core has dealt with what he said (and not merely finished something else while he talks)
            if case .listening = status {} else if case .sending = status {} else if !self.voice.micBusy { self.dictating = false; self.micOpen = false }
            // his words stay up while they are being sent, and go when the microphone has closed and they have gone
            if case .listening = status {} else if case .sending = status {} else { self.hearing = "" }
        }
    }
    func hearing(text: String) { DispatchQueue.main.async { self.hearing = text } }
    func words(text: String, starts: [UInt32], atMs: [UInt32]) {
        DispatchQueue.main.async {
            let more = zip(starts, atMs).map { (Int($0), Double($1) / 1000) }
            if text.isEmpty { self.caption?.marks.append(contentsOf: more) } else { self.caption = Caption(text: text, marks: more) }
        }
    }
    /// How far the voice has got in what it is saying, in characters, by the clock of the sound itself.
    func captionReached() -> Int {
        guard let caption, let position = voice.clipPosition() else { return 0 }
        return caption.reached(position)
    }
    func note(text: String) {
        // he may not be looking at the screen: it is heard as well as shown
        voice.uhOh()
        DispatchQueue.main.async { self.problem = text }
    }
    func watching() -> Bool {
        frontLock.lock()
        defer { frontLock.unlock() }
        return front
    }
}

/// Words being spoken, with their timings.
struct Caption {
    let text: String
    var marks: [(at: Int, time: Double)]

    /// How far into `text` the voice has got `t` seconds into the clip, in characters: the end of the word being said.
    func reached(_ t: Double) -> Int {
        guard let i = marks.lastIndex(where: { $0.time <= t }) else { return 0 }
        return i + 1 < marks.count ? marks[i + 1].at : text.count
    }
}
