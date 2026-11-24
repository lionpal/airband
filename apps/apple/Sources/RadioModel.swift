// Glue between the Rust core and the screen. The core's desk holds every paired computer as one and decides
// everything about the chat: what is waiting, what is done, where a message goes, who has the floor on a call.
// This file holds what the screens are drawn from (the desk's view and the phone's own state beside it) and
// keeps the pairing links in the Keychain. What he chooses is in +Choices, what he does with his agents in
// +Agents, the core's host calls (speaker, microphone, keys) in +Host, calling the core in +Calls, his computers
// in +Computers, the banners in +Banners, and what he attaches in AttachPrepare.
import AVFoundation
import Foundation
import ActivityKit
import SwiftUI
import UIKit

final class RadioModel: ObservableObject, DeskHost, @unchecked Sendable {
    static let shared = RadioModel()

    /// What the screen shows, as the desk last gave it, for the room or agent he is looking at.
    @Published var view = DeskView(paired: false, online: false, connecting: false, near: false, away: [], feed: [], rooms: [], unread: 0, severalComputers: false, unreadElsewhere: 0, canPoke: false, quick: [], sending: [], takes: [], stations: [], colours: [:], mainComputer: nil, chooseMain: false, uploads: [], asking: nil, intents: [], focus: DeskFocus(room: nil, agent: nil), opening: nil, stoppable: [], newAgent: NewAgentDefaults(computer: nil, tool: nil), tools: [], cannotReach: "", noWayIn: nil)
    /// A project: show just its agents, and keep what he says among them. The desk keeps it ([`look`]).
    var room: String? { view.focus.room }
    /// An agent, by id: show just that agent.
    var only: String? { view.focus.agent }
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
    /// Whether the kept pairing links have been read. Until they are, nothing is written over them.
    private var linksRead = false
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
        engines = [NeuralEngineListening.name: { [ownListening] folder, job, input in job == .ears ? ownListening.hear(folder: folder, pcm: input) : nil }]
        // the newest catalogue of services and models this phone has kept, before anything is listed from it
        loadCatalog(dir: dir.path)
        Self.protectTakes(dataDir: dir.path)
        desk = Desk.open(dataDir: dir.path, host: self)
        // a model's files come with the system's own transfers, which go on while the app is in the background
        ModelTransfers.shared.model = self
        ModelTransfers.shared.open()
        desk.useTransfers(transfers: ModelTransfers.shared)
        #if DEBUG
        // development only: `-airband.devTools '("<a tool id>")'` opens as someone who uses those coding tools, for a screenshot
        if let tools = Dev.list("devTools") { desk.setTools(tools: tools) }
        #endif
        var links: [String]? = readLinks()
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
        // links that could not be read before the phone was first unlocked are read once it is
        center.addObserver(forName: UIApplication.protectedDataDidBecomeAvailableNotification, object: nil, queue: .main) { [weak self] _ in
            guard let self, !self.linksRead, let links = self.readLinks() else { return }
            self.connect(links)
        }
        if let links { connect(links) }
    }

    /// The kept pairing links, or nil when the Keychain could not be read: then nothing is connected and nothing
    /// is written, and he is told.
    private func readLinks() -> [String]? {
        switch Keychain.links() {
        case .found(let links): linksRead = true; return links
        case .none: linksRead = true; return []
        case .failed: problem = RadioModel.linksUnread; return nil
        }
    }
    static let linksUnread = "Can\u{2019}t read your pairings yet. Unlock your phone."
    static let notALink = "That is not a pairing link. Run airband pair on your computer."
    static let keysUnread = "Can\u{2019}t read your keys yet. Unlock your phone and try again."
    static let keyNotKept = "Couldn\u{2019}t keep your key on this phone. Try again."

    /// The app was put away. Reading messages aloud goes on only where that is wanted with the screen off:
    /// through headphones or a car. From the phone's own speaker it stops.
    private func wentAway() {
        Preparing.whileAway()
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
    /// What was wrong with this link, in words for him, or nothing when it was taken.
    @discardableResult
    func pair(with link: String) -> String? {
        let link = link.trimmed
        let wrong: String?
        if !isPairingLink(link: link) {
            wrong = RadioModel.notALink
        } else if !linksRead {
            wrong = RadioModel.linksUnread
        } else {
            wrong = links.contains(link) ? nil : connect(links + [link])
        }
        if let wrong { problem = wrong }
        return wrong
    }

    func forgetAll() {
        guard linksRead else { problem = RadioModel.linksUnread; return }
        connect([])
    }

    /// Where this phone can be woken, as Apple gave it: every paired computer is told.
    func pushAddress(_ token: String) { desk.setPush(provider: .apns, token: token) }

    /// An answer given on a notification. Blocks until the computer has it, or says why it does not.
    func answerNow(station: String, message: String, text: String, tapped: Tapped?) -> AnswerOutcome { desk.answerNow(station: station, messageId: message, text: text, tapped: tapped) }

    /// Look at everyone, at one team, or at one agent. Whatever he was answering is let go: it may no longer be
    /// in view. The desk keeps what he is looking at, and lets go of a team that has gone.
    func look(room: String? = nil, agent: String? = nil) {
        replyingTo = nil
        desk.look(room: room, agent: agent)
        refresh()
    }
    /// A team he has just made: the desk already shows it, empty, until its first agent comes.
    func openMade() {
        replyingTo = nil
        refresh()
    }

    /// He tapped a notification: the chat is followed up to it, then it is shown on its agent and marked
    /// (`view.opening` says where it stands).
    func openMessage(station: String, message: String, rev: UInt64, room: String, agent: String) {
        replyingTo = nil
        desk.openMessage(station: station, message: message, rev: rev, room: room, agent: agent)
        refresh()
    }
    /// Done with the message opened from a notification.
    func closeOpening() {
        desk.closeOpening()
        refresh()
    }

    /// A team on his screen, by its room id.
    func room(_ id: String?) -> DeskRoom? { id.flatMap { id in view.rooms.first { $0.id == id } } }

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
    func fromAndComputer(_ m: ChatMessage) -> String { [m.from, m.computer].filter { !$0.isEmpty }.joined(separator: " · ") }
    /// What he is told when a computer cannot be reached: which, when more than one could be meant.
    var cannotReach: String { view.cannotReach }
    /// A state as it is read out for someone who cannot see its dot.
    static func spoken(state: AgentState) -> String { spokenState(state: state) }

    /// A message's words whole, once they have been fetched; as it came until then.
    func whole(_ m: ChatMessage) -> String { full[m.id] ?? m.text }

    /// Pair exactly these links. What went wrong, in words for him, or nothing.
    @discardableResult
    private func connect(_ wanted: [String]) -> String? {
        let paired = desk.setLinks(links: wanted)
        links = paired.links
        var wrong: String? = paired.unread
        if linksRead && !Keychain.keepLinks(links) { wrong = "Couldn\u{2019}t save your pairings on this phone." }
        if let wrong { problem = wrong }
        refresh()
        // paired: this is when notifications are worth asking for, and when a new computer needs the phone's address
        if !links.isEmpty { DispatchQueue.main.async { Push.shared.setUp() } }
        return wrong
    }

    func refresh() {
        // what the screen shows is in front of him: read, before it is drawn
        desk.shown()
        // only when something he can see has changed: the chat is not laid out again for nothing
        let now = desk.view()
        if now != view {
            // a team given its colour just now: the notification extension shows it the same
            if now.colours != view.colours { Together.keepTeamColours(now.colours) }
            view = now
        }
        letGoOfSent()
        // opened long ago: fetched again if he looks again. What is in the chat now stays, so a long message
        // on screen never falls back to its opening
        if full.count > (showWhole ? 400 : 60) {
            let shown = Set(view.feed.map(\.id))
            full = full.filter { shown.contains($0.key) }
        }
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
        look(room: team.id, agent: id)
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

    /// The banners up now, by the agent each is for (`RadioModel+Banners.swift`).
    var banners: [String: Activity<NeedsYouAttributes>] = [:]

    /// The coding tools he uses, by id. The core keeps them: what can write his summaries follows from them.
    var tools: [String] {
        get { desk.tools() }
        set {
            objectWillChange.send()
            desk.setTools(tools: newValue)
            refresh()
        }
    }
    /// Whose sign-in writes his summaries, among his coding tools, and whether he is to be asked which.
    var summaryTool: SummaryTool { desk.summaryTool() }
    func setSummaryTool(_ id: String) {
        objectWillChange.send()
        desk.setSummaryTool(tool: id)
        refresh()
    }


    /// Goes up when a newer catalogue of services and models has been taken: what the choosers list is read again.
    @Published var catalogRev = 0
    var catalogAsked = Date.distantPast

    /// What he can choose for each job (what writes his summaries, what listens, what speaks) and how each
    /// choice stands, as the core last said. The core decides all of it; the screens draw this.
    @Published var choices: [ChoiceJob: [Choice]] = [:]


    /// The app's own folder: the core keeps what he chose there, and the models that run on this phone.
    let voiceDir: String

    let keysLock = NSLock()

    /// The clip being played to let him hear a voice before choosing it, and which voice it is.
    @Published var previewing: String?
    /// The recording of something he said that is playing, by its id (`RadioModel+Takes.swift`).
    @Published var playingTake: String?

    /// The engines this app runs models with itself, by the name a model's row in the catalogue gives. The
    /// core offers a model for one of these only on a system its row allows, and asks for it by this name.
    /// Set once in `init`, before the core is opened and can ask from its own threads.
    private(set) var engines: [String: (String, ChoiceJob, Data) -> Data?] = [:]

    /// The listening model this app runs itself.
    let ownListening = NeuralEngineListening()

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

/// Told by a player when its clip has finished, or could not be played.
final class PlayerEnd: NSObject, AVAudioPlayerDelegate {
    var ended: () -> Void = {}
    func audioPlayerDidFinishPlaying(_ player: AVAudioPlayer, successfully flag: Bool) { DispatchQueue.main.async { self.ended() } }
    func audioPlayerDecodeErrorDidOccur(_ player: AVAudioPlayer, error: Error?) { DispatchQueue.main.async { self.ended() } }
}
