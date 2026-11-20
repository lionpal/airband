// Glue between the Rust core and the screen. The core's desk holds every paired computer as one and decides
// everything about the chat: what is waiting, what is done, where a message goes, who has the floor on a call.
// This file passes the desk's view to SwiftUI, lends it the phone's speaker and microphone, and keeps the
// pairing links in the Keychain. Nothing here would have to be written again in the same way for Android or
// Windows beyond these three things.
import AVFoundation
import Foundation
import Security
import SwiftUI
import UIKit

final class RadioModel: ObservableObject, DeskHost, @unchecked Sendable {
    static let shared = RadioModel()

    /// What the screen shows, as the desk last gave it, for the room or agent he is looking at.
    @Published var view = DeskView(paired: false, online: false, connecting: false, near: false, away: [], feed: [], rooms: [], canPoke: false, quick: [], sending: [], stations: [], uploads: [], asking: nil)
    /// A project: show just its agents, and keep what he says among them.
    @Published var room: String? { didSet { refresh() } }
    /// A callsign: show just that agent.
    @Published var only: String? { didSet { refresh() } }
    @Published var full: [String: String] = [:] // message id → exactly what the agent wrote
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
    @Published var said = "" // what the device's own voice is saying on a call
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

    private let voice = Voice()
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
        desk = Desk.open(dataDir: dir.path, host: self)
        var links = (try? JSONDecoder().decode([String].self, from: Data((Keychain.read("airband.links") ?? "[]").utf8))) ?? []
        if let old = Keychain.read("airband.link"), !old.isEmpty, !links.contains(old) { links.append(old) } // from the first builds
        #if DEBUG
        // development only: `xcrun simctl launch <sim> dev.airband.app -airband.devLink <pairing link>` pairs without the
        // system's "Open in airband?" prompt, which a script cannot tap
        if let dev = UserDefaults.standard.string(forKey: "airband.devLink"), dev.hasPrefix("airband://pair?") {
            links = dev.split(separator: " ").map(String.init) // several links, separated by spaces
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
        guard link.hasPrefix("airband://pair?") else {
            problem = "That is not a pairing link. Run airband pair on your computer."
            return
        }
        guard !links.contains(link) else { return }
        connect(links + [link])
    }

    func forgetAll() { connect([]) }

    /// Where this phone can be woken, as Apple gave it: every paired computer is told.
    func pushAddress(_ token: String) { desk.setPush(provider: "apns", token: token) }

    /// An answer given on a notification. Blocks until the computer has it, or says why it does not.
    func answerNow(station: String, message: String, text: String) throws { try desk.answerNow(station: station, messageId: message, text: text) }

    /// He tapped a notification: show that agent.
    func show(room: String, agent: String) {
        replyingTo = nil
        self.room = room.isEmpty ? nil : room
        only = agent.isEmpty ? nil : agent
    }

    private func connect(_ wanted: [String]) {
        links = desk.setLinks(links: wanted)
        if links.count < wanted.count { problem = "One pairing link is not valid and was skipped." }
        Keychain.write("airband.links", String(data: (try? JSONEncoder().encode(links)) ?? Data("[]".utf8), encoding: .utf8) ?? "[]")
        refresh()
        // paired: this is when notifications are worth asking for, and when a new computer needs the phone's address
        if !links.isEmpty { DispatchQueue.main.async { Push.shared.setUp() } }
    }

    private func refresh() {
        view = desk.view(room: room, agent: only)
        // what has gone with a message has left the desk's list: it leaves the tray too
        if !tray.isEmpty {
            let gone = tray.filter { item in item.attachment.map { id in !view.uploads.contains { $0.id == id } } ?? false }
            gone.forEach { if let source = $0.source { try? FileManager.default.removeItem(at: source) } }
            tray.removeAll { item in gone.contains { $0.id == item.id } }
        }
        // a room that has gone (its last agent left) is not somewhere he can stay
        if let room, view.online, !view.rooms.contains(where: { $0.name == room }) { self.room = nil }
        if full.count > 60 { full.removeAll() } // opened long ago: fetched again if he looks again
    }

    /// One tap on an answer button: send it as the answer to that message.
    func answer(_ m: ChatMessage, _ choice: String) {
        if replyingTo?.id == m.id { replyingTo = nil }
        desk.answer(messageId: m.id, text: choice)
    }

    /// Send what he typed: to the message he picked if he picked one, else to whoever the desk and the tower
    /// work out from his words and where he is looking.
    func send(_ text: String) {
        let target = replyingTo
        replyingTo = nil
        // with nothing typed there is still something to send when files are attached: the files themselves
        let words = text.trimmingCharacters(in: .whitespacesAndNewlines)
        desk.send(text: words.isEmpty && !tray.isEmpty ? "Look at what I attached." : text, re: target?.id, room: room, agent: only)
    }

    /// Try the computers again, now.
    func reconnect() {
        DispatchQueue.global(qos: .userInitiated).async { self.desk.reconnect() }
    }

    /// The agents he could stop right now: at work, run by their computer, and in the room or agent he is looking at.
    var stoppable: [DeskMember] {
        view.rooms.filter { room == nil || $0.name == room }.flatMap(\.members).filter { $0.held && $0.state == "working" && (only == nil || $0.callsign == only) }
    }

    /// Stop what an agent is doing, now. What he said to it meanwhile it then reads together.
    func stop(_ agent: DeskMember) {
        fetch.async {
            let result = Result { try self.desk.stopAgent(agent: agent.id) }
            DispatchQueue.main.async {
                switch result {
                case .success: break
                case .failure(RadioError.Told(let reason)): self.problem = reason
                case .failure(RadioError.Offline): self.problem = "Cannot reach that computer."
                case .failure: self.problem = "It did not stop."
                }
            }
        }
    }

    /// End an agent its computer runs: it finishes and leaves the radio.
    func end(_ agent: DeskMember) {
        fetch.async {
            let result = Result { try self.desk.endAgent(agent: agent.id) }
            DispatchQueue.main.async {
                switch result {
                case .success: if self.only == agent.callsign { self.only = nil }
                case .failure(RadioError.Told(let reason)): self.problem = reason
                case .failure(RadioError.Offline): self.problem = "Cannot reach that computer."
                case .failure: self.problem = "It did not end."
                }
            }
        }
    }

    /// Take back something he said that is still waiting its turn.
    func unsend(_ m: ChatMessage) {
        fetch.async {
            let result = Result { try self.desk.unsend(messageId: m.id) }
            DispatchQueue.main.async {
                if case .failure(RadioError.Told(let reason)) = result { self.problem = reason }
            }
        }
    }

    /// Have its computer take over an agent he started in a terminal, so his words reach it as his own.
    func takeOver(_ agent: DeskMember) {
        fetch.async {
            let result = Result { try self.desk.takeOver(agent: agent.id) }
            DispatchQueue.main.async {
                switch result {
                case .success: break
                case .failure(RadioError.Told(let reason)): self.problem = reason
                case .failure(RadioError.Offline): self.problem = "Cannot reach that computer."
                case .failure: self.problem = "It was not taken over."
                }
            }
        }
    }

    /// Send the words of one of his messages again, to another agent.
    func resend(_ m: ChatMessage, to agent: DeskMember) {
        if m.long { loadFull(m) }
        desk.send(text: "@\(agent.callsign.replacingOccurrences(of: " ", with: "")) \(full[m.id] ?? m.text)", re: nil, room: nil, agent: nil)
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

    /// Read a whole message aloud, in the voice of the computer it came from, or stop if it is the one being read.
    func readAloud(_ m: ChatMessage) {
        if reading == m.id {
            stopReading()
            return
        }
        stopReading()
        reading = m.id
        DispatchQueue.global(qos: .userInitiated).async {
            let read = self.desk.readAloud(messageId: m.id)
            DispatchQueue.main.async {
                guard self.reading == m.id else { return }
                self.reading = nil
                if !read { self.problem = "That computer has no voice to read with." }
            }
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
        fetch.async {
            let result = Result { try self.desk.useAccount(station: station, harness: a.harness, id: a.id) }
            DispatchQueue.main.async {
                self.switching = nil
                switch result {
                case .success(let now):
                    if let i = self.accounts.firstIndex(where: { $0.station == station }) { self.accounts[i] = now }
                case .failure(RadioError.Told(let reason)): self.accountsProblem = reason
                case .failure(RadioError.Offline): self.accountsProblem = "Cannot reach that computer."
                case .failure: self.accountsProblem = "It did not switch."
                }
            }
        }
    }

    /// Put an agent in another team: one that is there, or a new one of that name. If he was looking at that
    /// agent alone, he still is, under its new name.
    func moveAgent(_ agent: DeskMember, to team: String) {
        let team = team.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !team.isEmpty else { return }
        let watched = only == agent.callsign
        fetch.async {
            let result = Result { try self.desk.moveAgent(agent: agent.id, team: team) }
            DispatchQueue.main.async {
                switch result {
                case .success:
                    if watched { self.only = self.view.rooms.flatMap(\.members).first { $0.id == agent.id }?.callsign }
                case .failure(RadioError.Told(let reason)): self.problem = reason
                case .failure(RadioError.Offline): self.problem = "Cannot reach that computer."
                case .failure: self.problem = "It did not move."
                }
            }
        }
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
            said = ""
        } else {
            voice.goOffDuty()
        }
        calling = on
        desk.setCallMode(on: on)
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
    func status(status: RadioStatus, saying: String) {
        DispatchQueue.main.async {
            self.status = status
            if !saying.isEmpty { self.said = saying }
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
