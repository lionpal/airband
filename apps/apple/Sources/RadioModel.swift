// Glue between the Rust core and the screen. The core's desk holds every paired computer as one and decides
// everything about the chat: what is waiting, what is done, where a message goes, who has the floor on a call.
// This file passes the desk's view to SwiftUI, lends it the phone's speaker and microphone, and keeps the
// pairing links in the Keychain. Nothing here would have to be written again in the same way for Android or
// Windows beyond these three things.
import Foundation
import Security
import SwiftUI
import UIKit

final class RadioModel: ObservableObject, DeskHost, @unchecked Sendable {
    static let shared = RadioModel()

    /// What the screen shows, as the desk last gave it, for the room or agent he is looking at.
    @Published var view = DeskView(paired: false, online: false, feed: [], rooms: [], canPoke: false, quick: [], sending: [], stations: [])
    /// A project: show just its agents, and keep what he says among them.
    @Published var room: String? { didSet { refresh() } }
    /// A callsign: show just that agent.
    @Published var only: String? { didSet { refresh() } }
    @Published var full: [String: String] = [:] // message id → exactly what the agent wrote
    @Published var pictureFiles: [String: String] = [:] // picture id → the file it was fetched into
    @Published var replyingTo: ChatMessage?
    @Published var problem: String?
    // call mode
    @Published var calling = false
    @Published var status: RadioStatus = .idle
    @Published var said = "" // what the device's own voice is saying on a call
    @Published var reading: String? // the message being read aloud from its full text, by id
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
        center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in self?.setFront(true) }
        center.addObserver(forName: UIApplication.willResignActiveNotification, object: nil, queue: .main) { [weak self] _ in self?.setFront(false) }
        connect(links)
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

    private func connect(_ wanted: [String]) {
        links = desk.setLinks(links: wanted)
        if links.count < wanted.count { problem = "One pairing link is not valid and was skipped." }
        Keychain.write("airband.links", String(data: (try? JSONEncoder().encode(links)) ?? Data("[]".utf8), encoding: .utf8) ?? "[]")
        refresh()
    }

    private func refresh() {
        view = desk.view(room: room, agent: only)
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
        desk.send(text: text, re: target?.id, room: room, agent: only)
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

    /// Read a whole message aloud in the phone's own voice, or stop if it is the one being read.
    func readAloud(_ m: ChatMessage, _ text: String) {
        if reading == m.id {
            stopReading()
            return
        }
        voice.hush()
        reading = m.id
        fetch.async {
            self.voice.speak(text)
            DispatchQueue.main.async { if self.reading == m.id { self.reading = nil } }
        }
    }

    func stopReading() {
        guard reading != nil else { return }
        reading = nil
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
    @MainActor func talk(about message: ChatMessage? = nil) async {
        if let missing = await voice.authorize() {
            problem = missing
            return
        }
        // `message`: he is looking at something of one agent's (a picture, say) and this is about that
        let target = message ?? replyingTo
        replyingTo = nil
        desk.talk(re: target?.id, room: only == nil ? room : nil)
    }

    // MARK: DeskHost: called by the core, on its own threads

    func speak(text: String) { voice.speak(text) }
    func listen() -> String? { voice.listen() }
    func play(rate: UInt32, pcm: Data, last: Bool) { voice.play(rate: rate, pcm: pcm, last: last) }
    func recordStart(rate: UInt32) -> Bool { voice.recordStart(rate: rate) }
    func recordRead() -> Data { voice.recordRead() }
    func recordStop() { voice.recordStop() }
    func changed() { DispatchQueue.main.async { self.refresh() } }
    func status(status: RadioStatus, saying: String) {
        DispatchQueue.main.async {
            self.status = status
            if !saying.isEmpty { self.said = saying }
        }
    }
    func note(text: String) { DispatchQueue.main.async { self.problem = text } }
    func watching() -> Bool {
        frontLock.lock()
        defer { frontLock.unlock() }
        return front
    }
}

enum Keychain {
    static func write(_ key: String, _ value: String) {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrAccount as String: key]
        SecItemDelete(query as CFDictionary)
        var add = query
        add[kSecValueData as String] = Data(value.utf8)
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock // readable with the screen locked, while driving
        SecItemAdd(add as CFDictionary, nil)
    }
    static func read(_ key: String) -> String? {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrAccount as String: key, kSecReturnData as String: true]
        var out: AnyObject?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }
}
