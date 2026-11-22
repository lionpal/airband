// Glue between the Rust core and the screen. The core's desk holds every paired computer as one and decides
// everything about the chat: what is waiting, what is done, where a message goes, who has the floor on a call.
// This file passes the desk's view to SwiftUI, lends it the phone's speaker and microphone, and keeps the
// pairing links in the Keychain. Nothing here would have to be written again in the same way for Android or
// Windows beyond these three things. What he chooses, what he does with his agents, and the core's host calls are
// in RadioModel+Choices, +Agents and +Host.
import AVFoundation
import PhononCoreML
import Foundation
import ActivityKit
import SwiftUI
import UIKit

final class RadioModel: ObservableObject, DeskHost, @unchecked Sendable {
    static let shared = RadioModel()

    /// What the screen shows, as the desk last gave it, for the room or agent he is looking at.
    @Published var view = DeskView(paired: false, online: false, connecting: false, near: false, away: [], feed: [], rooms: [], unread: 0, canPoke: false, quick: [], sending: [], stations: [], colours: [:], mainComputer: nil, chooseMain: false, uploads: [], asking: nil)
    /// A project: show just its agents, and keep what he says among them.
    @Published var room: String? { didSet { refresh() } }
    /// An agent, by id: show just that agent.
    @Published var only: String? { didSet { refresh() } }
    /// What the agent he is looking at is called now.
    var onlyName: String? { only.flatMap { member($0)?.agent.callsign } }
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
    var dictating = false // from his tap until the core has dealt with what he said
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
    /// The account being switched to: on which computer, and which account.
    struct Switching: Equatable { let station: String; let account: String }
    @Published var switching: Switching?
    @Published var accountsProblem: String?

    let voice = Voice()
    var desk: Desk!
    private var links: [String] = []
    var fetching: Set<String> = []
    /// Fetching full text and pictures: a few at a time on shared threads, not a thread each.
    let fetch = DispatchQueue(label: "dev.airband.fetch", qos: .userInitiated, attributes: .concurrent)
    /// Whether the app is in front of him, readable from any thread.
    let frontLock = NSLock()
    var front = true

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
        if let tools = Dev.list("devTools") { desk.setTools(tools: tools) }
        #endif
        var links = Keychain.links()
        #if DEBUG
        // development only: `xcrun simctl launch <sim> dev.airband.app -airband.devLink <pairing link>` pairs without the
        // system's "Open in airband?" prompt, which a script cannot tap
        if let dev = Dev.text("devLink"), isPairingLink(link: dev.split(separator: " ").first.map(String.init) ?? "") {
            links = dev.split(separator: " ").map(String.init) // several links, separated by spaces
        }
        #endif
        #if DEBUG
        // development only: `-airband.devMic YES` opens the microphone a few seconds after launch, as a tap on
        // the big button would, for a run nobody is there to tap
        // the models he chose are fetched first, the microphone is open for eight seconds, and what it heard is sent
        if Dev.flag("devMic") {
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
    func pushAddress(_ token: String) { desk.setPush(provider: .apns, token: token) }

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
    /// An agent, by its id, with the team it is in: the one way the screens find which team an agent is in.
    func member(_ id: String) -> (agent: DeskMember, team: DeskRoom)? {
        for room in view.rooms {
            if let member = room.members.first(where: { $0.id == id }) { return (member, room) }
        }
        return nil
    }
    /// An agent's badge and its team's colour, by the agent's id.
    func badge(of agent: String) -> (String, UInt8?)? {
        member(agent).map { ($0.agent.badge, $0.team.colour) }
    }
    /// His computers, each with what it is called to him.
    var computers: [DeskComputer] { view.stations }
    /// What a computer is called to him, by its id.
    func computerName(_ id: String) -> String { view.stations.first { $0.id == id }?.name ?? "" }
    /// Who a message is from, and on which computer when he has several.
    func fromAndComputer(_ m: ChatMessage) -> String { [m.from, computerName(m.station)].filter { !$0.isEmpty }.joined(separator: " · ") }
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
        Keychain.keepLinks(links)
        refresh()
        // paired: this is when notifications are worth asking for, and when a new computer needs the phone's address
        if !links.isEmpty { DispatchQueue.main.async { Push.shared.setUp() } }
    }

    func refresh() {
        // only when something he can see has changed: the chat is not laid out again for nothing
        let now = desk.view(room: room, agent: only)
        if now != view {
            // a team given its colour just now: the notification extension shows it the same
            if now.colours != view.colours { Keychain.keepTeamColours(now.colours) }
            view = now
        }
        // what has gone with a message has left the desk's list: it leaves the tray too
        if !tray.isEmpty {
            let gone = tray.filter { item in item.attachment.map { id in !view.uploads.contains { $0.id == id } } ?? false }
            gone.forEach { if let source = $0.source { try? FileManager.default.removeItem(at: source) } }
            tray.removeAll { item in gone.contains { $0.id == item.id } }
        }
        // a room that has gone (its last agent left) is not somewhere he can stay
        if let room, view.online, !view.rooms.contains(where: { $0.id == room }) { self.room = nil }
        if full.count > (showWhole ? 400 : 60) { full.removeAll() } // opened long ago: fetched again if he looks again
        refreshChoices()
        syncBanners()
    }

    /// Open the microphone for him to talk, unless it is open already; with a question, about that question.
    func openMic(about question: ChatMessage? = nil) {
        Task { @MainActor in if !self.micOpen { await self.talk(about: question) } }
    }

    /// An address the app was opened with: a pairing link, or what the Talk tile and a banner open it with.
    func open(_ url: URL) {
        switch OpenWith.route(url) {
        case .talk:
            replyingTo = nil
            openMic()
        case .answer(let agent?):
            answer(agent)
        case .answer(nil):
            openMic()
        case nil:
            pair(with: url.absoluteString)
        }
    }

    /// Answer an agent that is waiting on him, hands free: its question is read out again, with the answers
    /// it offers, and then the microphone opens. What he says is the answer to that question, in whatever
    /// words he uses.
    func answer(_ id: String) {
        guard let found = member(id) else { return }
        let (agent, team) = found
        // look at that agent, so the question is in view and his words go to it
        replyingTo = nil
        room = team.id
        only = id
        let question = view.feed.last { $0.id == agent.questionId }
        guard !agent.questionId.isEmpty else {
            openMic()
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
                self.openMic(about: question)
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

    /// The picture he chose for each of his computers, by its id: a system symbol's name.
    @Published var computerIcons: [String: String] = Stored.settings.load("airband.computers.icons.byid", or: [:])
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
        Stored.settings.keep("airband.computers.icons.byid", computerIcons)
    }

    /// Goes up when a newer catalogue of services and models has been taken: what the choosers list is read again.
    @Published var catalogRev = 0
    var catalogAsked = Date.distantPast

    /// What he can choose for each job (what writes his summaries, what listens, what speaks) and how each
    /// choice stands, as the core last said. The core decides all of it; the screens draw this.
    @Published var choices: [ChoiceJob: [Choice]] = [:]

    /// What he is told when a computer could not be asked.
    static let unreached = "Cannot reach that computer."

    /// What went wrong, in words for him: the reason the computer gave when it gave one, that it cannot be
    /// reached when it cannot, and otherwise `fallback`, which says what did not happen.
    static func words(for error: Error, else fallback: String) -> String {
        switch error {
        case RadioError.Told(let reason), RadioError.Kept(let reason): return reason
        case RadioError.Offline: return unreached
        default: return fallback
        }
    }

    /// Run one of the core's blocking calls off the main thread and come back on it: with what the call gave,
    /// or with what went wrong in words. Every call that can fail goes through here.
    func ask<T>(_ fallback: String, _ call: @escaping () throws -> T, then: @escaping (T?, String?) -> Void) {
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
    func tell(_ fallback: String, _ call: @escaping () throws -> Void, done: @escaping () -> Void = {}) {
        ask(fallback, call) { _, wrong in
            if let wrong { self.problem = wrong } else { done() }
        }
    }

    /// One of the core's calls that changes what is chosen. Calls back with what it said was wrong.
    func work(_ done: @escaping (String?) -> Void, _ call: @escaping () throws -> Void) {
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

    /// The app's own folder: the core keeps what he chose there, and the models that run on this phone.
    let voiceDir: String

    let keysLock = NSLock()

    /// The clip being played to let him hear a voice before choosing it, and which voice it is.
    @Published var previewing: String?
    var previewPlayer: AVAudioPlayer?

    /// The engines this app runs models with itself, by the name a model's row in the catalogue gives. The
    /// core offers a model for one of these only on a system its row allows, and asks for it by this name.
    lazy var engines: [String: (String, ChoiceJob, Data) -> Data?] = ["phonon-coreml": { [unowned self] in self.phononCoreML(folder: $0, job: $1, input: $2) }]

    /// The models loaded so far, by the folder each is in. Loading one takes a moment, so it is kept.
    var heardWith: [String: Transcriber] = [:]
    let hearLock = NSLock()

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
