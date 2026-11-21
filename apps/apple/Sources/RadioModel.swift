// Glue between the Rust core and the screen. The core's desk holds every paired computer as one and decides
// everything about the chat: what is waiting, what is done, where a message goes, who has the floor on a call.
// This file passes the desk's view to SwiftUI, lends it the phone's speaker and microphone, and keeps the
// pairing links in the Keychain. Nothing here would have to be written again in the same way for Android or
// Windows beyond these three things.
import AVFoundation
import PhononCoreML
import Foundation
import Security
import ActivityKit
import SwiftUI
import UIKit

final class RadioModel: ObservableObject, DeskHost, @unchecked Sendable {
    static let shared = RadioModel()

    /// What the screen shows, as the desk last gave it, for the room or agent he is looking at.
    @Published var view = DeskView(paired: false, online: false, connecting: false, near: false, away: [], feed: [], rooms: [], unread: 0, canPoke: false, quick: [], sending: [], stations: [], keyed: [], reads: "", uploads: [], asking: nil)
    /// A project: show just its agents, and keep what he says among them.
    @Published var room: String? { didSet { refresh() } }
    /// A callsign: show just that agent.
    @Published var only: String? { didSet { refresh() } }
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
        // A new install starts with the models that run on this phone chosen for listening and for the voice:
        // they need nothing from him, and are fetched when he finishes the first run. Someone who was already
        // using the app keeps what he had.
        let defaults = UserDefaults.standard
        if !defaults.bool(forKey: "airband.onboarded") && Keychain.read("airband.links") == nil && Keychain.read("airband.link") == nil {
            if defaults.string(forKey: "airband.voice") == nil, let first = localVoices(dir: dir.path).first { defaults.set(first.kind, forKey: "airband.voice") }
            if defaults.string(forKey: "airband.ears.local") == nil, let first = localListeners(dir: dir.path).first { defaults.set(first.kind, forKey: "airband.ears.local") }
        }
        setLocalVoice(dir: dir.path, kind: UserDefaults.standard.string(forKey: "airband.voice") ?? "", voice: UserDefaults.standard.string(forKey: "airband.voice.name") ?? "")
        setLocalEars(dir: dir.path, kind: UserDefaults.standard.string(forKey: "airband.ears.local") ?? "")
        if !serviceVoice.isEmpty { setServiceVoice(kind: serviceVoice, key: phoneKeys[serviceVoice] ?? "", settings: serviceSettings) }
        if !ownEars.isEmpty { setOwnEars(kind: ownEars, key: phoneKeys[ownEars] ?? "", settings: ownEarsSettings) }
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
        sendPendingReads()
        syncBanners()
    }

    /// An address the app was opened with: a pairing link, or what the Talk tile and a banner open it with.
    func open(_ url: URL) {
        switch url.host {
        case "talk":
            replyingTo = nil
            Task { @MainActor in if !self.micOpen { await self.talk() } }
        case "answer":
            if let callsign = OpenWith.agent(in: url) { answer(callsign) } else { Task { @MainActor in if !self.micOpen { await self.talk() } } }
        default:
            pair(with: url.absoluteString)
        }
    }

    /// Answer an agent that is waiting on him, hands free: its question is read out again, with the answers
    /// it offers, and then the microphone opens. What he says is the answer to that question, in whatever
    /// words he uses.
    func answer(_ callsign: String) {
        guard let team = view.rooms.first(where: { $0.members.contains { $0.callsign == callsign } }), let agent = team.members.first(where: { $0.callsign == callsign }) else { return }
        // look at that agent, so the question is in view and his words go to it
        replyingTo = nil
        room = team.name
        only = callsign
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
        let waiting = calling ? view.rooms.flatMap(\.members).filter { $0.state == "asking" } : []
        for (callsign, banner) in banners where !waiting.contains(where: { $0.callsign == callsign }) {
            banners[callsign] = nil
            Task { await banner.end(nil, dismissalPolicy: .immediate) }
        }
        guard !waiting.isEmpty, ActivityAuthorizationInfo().areActivitiesEnabled else { return }
        for agent in waiting where banners[agent.callsign] == nil {
            let state = NeedsYouAttributes.ContentState(waiting: waiting.count, question: agent.question)
            guard let banner = try? Activity.request(attributes: NeedsYouAttributes(callsign: agent.callsign, badge: agent.badge), content: .init(state: state, staleDate: nil)) else { continue }
            banners[agent.callsign] = banner
            // the alert is what makes it show itself, and not only sit on the lock screen
            Task { await banner.update(.init(state: state, staleDate: nil), alertConfiguration: .init(title: "\(agent.callsign) needs you", body: LocalizedStringResource(stringLiteral: agent.question.isEmpty ? "Tap to answer" : agent.question), sound: .default)) }
        }
    }

    /// The coding tools he uses, by id, the one he uses most first. Someone who had the app before there was a
    /// choice uses Claude Code.
    @Published var tools: [String] = UserDefaults.standard.stringArray(forKey: "airband.tools") ?? ["claude-code"] {
        didSet { UserDefaults.standard.set(tools, forKey: "airband.tools") }
    }

    /// The picture he chose for each of his computers, by its name: a system symbol's name.
    @Published var computerIcons: [String: String] = (try? JSONDecoder().decode([String: String].self, from: Data((UserDefaults.standard.string(forKey: "airband.computers.icons") ?? "{}").utf8))) ?? [:]
    /// The pictures a computer can have.
    static let computerSymbols = ["laptopcomputer", "macmini", "desktopcomputer", "macstudio", "macpro.gen3", "server.rack", "pc"]

    func icon(of computer: String) -> String { computerIcons[computer] ?? "desktopcomputer" }
    func setIcon(_ symbol: String, of computer: String) {
        computerIcons[computer] = symbol
        UserDefaults.standard.set(String(data: (try? JSONEncoder().encode(computerIcons)) ?? Data("{}".utf8), encoding: .utf8), forKey: "airband.computers.icons")
    }

    /// The folders inside one of his on a computer. Calls back with them, or with what is wrong.
    func folders(on computer: String, under: String, done: @escaping (DeskFolders?, String?) -> Void) {
        fetch.async {
            var (found, wrong): (DeskFolders?, String?) = (nil, nil)
            do { found = try self.desk.folders(computer: computer, under: under) } catch RadioError.Told(let reason) { wrong = reason } catch { wrong = "Cannot reach that computer." }
            DispatchQueue.main.async { done(found, wrong) }
        }
    }

    /// Make a new team on a computer. Calls back with what is wrong, or nil.
    func newTeam(_ name: String, on computer: String, folder: String, make: Bool, tool: String?, done: @escaping (String?) -> Void) {
        fetch.async {
            var wrong: String?
            do { try self.desk.newTeam(computer: computer, name: name, folder: folder, make: make, tool: tool) } catch RadioError.Told(let reason) { wrong = reason } catch { wrong = "Cannot reach that computer." }
            DispatchQueue.main.async { done(wrong) }
        }
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

    /// What he chose to write his summaries before any computer was paired: its kind. Sent when one is.
    @Published var pendingReads: String? = UserDefaults.standard.string(forKey: "airband.reads.pending")
    private var sendingReads = false

    /// Choose what reads the agents' messages. The work is done on his computer, so the choice goes there,
    /// with its key sealed; the computer tries it and keeps the key encrypted. With no computer paired yet the
    /// choice waits (its key in this phone's Keychain) and is sent when one is. Calls back with what is wrong.
    func chooseReads(_ kind: String, key: String?, settings: [String: String], done: @escaping (String?) -> Void) {
        if let key, !key.isEmpty { keepPhoneKey(kind, key) }
        guard view.online else {
            UserDefaults.standard.set(kind, forKey: "airband.reads.pending")
            UserDefaults.standard.set(String(data: (try? JSONEncoder().encode(settings)) ?? Data("{}".utf8), encoding: .utf8), forKey: "airband.reads.pending.settings")
            pendingReads = kind
            return done(nil)
        }
        let key = key ?? phoneKeys[kind] ?? ""
        fetch.async {
            var wrong: String?
            do { try self.desk.setReads(kind: kind, key: key, settings: settings) } catch RadioError.Told(let reason) { wrong = reason } catch { wrong = "Your computer did not take that. Try again when it can be reached." }
            DispatchQueue.main.async {
                if wrong == nil {
                    UserDefaults.standard.removeObject(forKey: "airband.reads.pending")
                    self.pendingReads = nil
                    self.refresh()
                }
                done(wrong)
            }
        }
    }

    /// A computer can be reached and a choice was waiting for one: send it now, once.
    private func sendPendingReads() {
        guard let kind = pendingReads, view.online, !sendingReads else { return }
        sendingReads = true
        let settings = (try? JSONDecoder().decode([String: String].self, from: Data((UserDefaults.standard.string(forKey: "airband.reads.pending.settings") ?? "{}").utf8))) ?? [:]
        chooseReads(kind, key: nil, settings: settings) { problem in
            self.sendingReads = false
            // one it would not take is let go, and said: he can pick again in Settings
            if let problem {
                UserDefaults.standard.removeObject(forKey: "airband.reads.pending")
                self.pendingReads = nil
                self.problem = problem
            }
        }
    }

    /// Fetch the models he chose that run on this phone and are not here yet, one after another, and put each
    /// to use. `step` is told what is happening and how far along it is (nil when that cannot be said).
    func fetchChosen(step: @escaping (String?, Double?) -> Void, done: @escaping (String?) -> Void) {
        let voice = localVoices(dir: voiceDir).first { $0.kind == localVoice && !$0.fetched && serviceVoice.isEmpty }
        let ears = localListeners(dir: voiceDir).first { $0.kind == localEars && ownEars.isEmpty }
        let listen = {
            guard let ears else { return done(nil) }
            let ready = {
                step("Getting \(ears.name) ready for this phone. About a minute, once.", nil)
                self.prepareLocalEars(ears.kind) { problem in
                    if problem == nil { self.useLocalEars(ears.kind) }
                    done(problem)
                }
            }
            if ears.fetched { return ready() }
            step("Downloading \(ears.name)", 0)
            self.downloadVoice(ears.kind) { problem in if let problem { done(problem) } else { ready() } }
        }
        guard let voice else { return listen() }
        step("Downloading \(voice.name)", 0)
        downloadVoice(voice.kind) { problem in
            if let problem { return done(problem) }
            self.useVoice(voice.kind)
            listen()
        }
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

    /// Where a voice made on this phone keeps its model.
    let voiceDir: String
    /// The voice made on this phone that he has chosen, or empty for the computer's.
    @Published var localVoice = UserDefaults.standard.string(forKey: "airband.voice") ?? ""

    /// The keys he has given this phone, by provider: one key serves everything its provider does here (its
    /// voice and its listening). Kept in the Keychain, on this phone only, and never shown again.
    var phoneKeys: [String: String] {
        (try? JSONDecoder().decode([String: String].self, from: Data((Keychain.read("airband.keys") ?? "{}").utf8))) ?? [:]
    }
    func keepPhoneKey(_ provider: String, _ key: String?) {
        var all = phoneKeys
        all[provider] = key
        Keychain.write("airband.keys", String(data: (try? JSONEncoder().encode(all)) ?? Data("{}".utf8), encoding: .utf8) ?? "{}")
        objectWillChange.send()
    }

    /// The voice service this phone speaks through with his own key, or empty.
    @Published var serviceVoice = UserDefaults.standard.string(forKey: "airband.voice.service") ?? ""
    /// What that service was told beyond the key.
    var serviceSettings: [String: String] {
        (try? JSONDecoder().decode([String: String].self, from: Data((UserDefaults.standard.string(forKey: "airband.voice.settings") ?? "{}").utf8))) ?? [:]
    }

    /// Try a voice service with a key and, if it speaks, use it from now on. `key` nil uses the one already kept
    /// for that provider. Calls back with what is wrong, or nil.
    func useService(_ kind: String, key: String?, settings: [String: String], done: @escaping (String?) -> Void) {
        let key = key?.trimmingCharacters(in: .whitespaces) ?? phoneKeys[kind] ?? ""
        fetch.async {
            var wrong: String?
            do { try checkServiceVoice(kind: kind, key: key, settings: settings) } catch RadioError.Told(let reason) { wrong = reason } catch { wrong = "Could not check the voice." }
            DispatchQueue.main.async {
                if wrong == nil {
                    if !key.isEmpty { self.keepPhoneKey(kind, key) }
                    UserDefaults.standard.set(kind, forKey: "airband.voice.service")
                    UserDefaults.standard.set(String(data: (try? JSONEncoder().encode(settings)) ?? Data("{}".utf8), encoding: .utf8), forKey: "airband.voice.settings")
                    self.serviceVoice = kind
                    // one voice at a time: the one made on the phone steps aside
                    self.useVoice("")
                    setServiceVoice(kind: kind, key: key, settings: settings)
                }
                done(wrong)
            }
        }
    }

    func stopService() {
        UserDefaults.standard.set("", forKey: "airband.voice.service")
        serviceVoice = ""
        setServiceVoice(kind: "", key: "", settings: [:])
    }

    /// Which of the phone's own voices speaks, by name. Empty is the usual one.
    @Published var voiceName = UserDefaults.standard.string(forKey: "airband.voice.name") ?? ""

    /// Speak in this one of the model's voices from now on. Takes effect at once when the model is in use.
    func chooseVoice(_ name: String) {
        UserDefaults.standard.set(name, forKey: "airband.voice.name")
        voiceName = name
        if !localVoice.isEmpty { setLocalVoice(dir: voiceDir, kind: localVoice, voice: name) }
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

    /// Speak with a voice made on this phone from now on; empty goes back to the computer's.
    func useVoice(_ kind: String) {
        UserDefaults.standard.set(kind, forKey: "airband.voice")
        localVoice = kind
        setLocalVoice(dir: voiceDir, kind: kind, voice: voiceName)
        if !kind.isEmpty { stopService() }
    }

    /// Fetch a voice's model. Calls back with what went wrong, or nil. The model can be fetched again, so it
    /// is left out of the phone's backups.
    func downloadVoice(_ kind: String, done: @escaping (String?) -> Void) {
        fetch.async {
            var wrong: String?
            do { try fetchLocalModel(dir: self.voiceDir, kind: kind) } catch RadioError.Told(let reason) { wrong = reason } catch { wrong = "The download did not finish." }
            var folder = URL(fileURLWithPath: self.voiceDir).appendingPathComponent("models")
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try? folder.setResourceValues(values)
            DispatchQueue.main.async { done(wrong) }
        }
    }

    func removeVoice(_ kind: String) {
        if localVoice == kind { useVoice("") }
        if localEars == kind { useLocalEars("") }
        heardWith.removeAll()
        removeLocalModel(dir: voiceDir, kind: kind)
    }

    /// Say a sentence in the chosen voice, so he hears what he picked.
    func sampleVoice(done: @escaping (String?) -> Void) {
        fetch.async {
            var wrong: String?
            do {
                let sample = try ownVoiceSample()
                self.voice.play(rate: sample.rate, pcm: sample.pcm, last: true)
            } catch RadioError.Told(let reason) { wrong = reason } catch { wrong = "The voice did not speak." }
            DispatchQueue.main.async { done(wrong) }
        }
    }

    /// The listening model on this phone that he has chosen, or empty.
    @Published var localEars = UserDefaults.standard.string(forKey: "airband.ears.local") ?? ""
    /// How long the model took over the last thing it heard, and how long that recording was, in seconds.
    @Published var lastHeard: (took: Double, of: Double)?

    /// Listen with a model on this phone from now on; empty goes back to a service or the computer.
    func useLocalEars(_ kind: String) {
        UserDefaults.standard.set(kind, forKey: "airband.ears.local")
        localEars = kind
        setLocalEars(dir: voiceDir, kind: kind)
    }

    /// Get a listening model ready on this phone: the first time, the phone spends about a minute fitting it to
    /// its own chip. Calls back with what went wrong, or nil.
    func prepareLocalEars(_ kind: String, done: @escaping (String?) -> Void) {
        let folder = URL(fileURLWithPath: voiceDir).appendingPathComponent("models").appendingPathComponent(kind).path
        fetch.async {
            let heard = self.transcribe(folder: folder, pcm: Data(count: 32000))
            DispatchQueue.main.async { done(heard == nil ? "The model did not load on this phone." : nil) }
        }
    }

    /// The models loaded so far, by the folder each is in. Loading one takes a moment, so it is kept.
    private var heardWith: [String: Transcriber] = [:]
    private let hearLock = NSLock()

    /// The core asks for a recording to be turned into words by a model on this phone (16 kHz, 16-bit, mono).
    func transcribe(folder: String, pcm: Data) -> String? {
        hearLock.lock()
        defer { hearLock.unlock() }
        do {
            let model = try heardWith[folder] ?? Transcriber(bundle: URL(fileURLWithPath: folder))
            heardWith[folder] = model
            let samples: [Float] = pcm.withUnsafeBytes { raw in raw.bindMemory(to: Int16.self).map { Float($0) / 32768 } }
            let began = Date()
            let heard = try model.transcribe(samples)
            let took = Date().timeIntervalSince(began)
            DispatchQueue.main.async { self.lastHeard = (took, heard.audioSeconds) }
            return heard.text
        } catch {
            NSLog("airband: the listening model failed: \(error.localizedDescription)")
            return nil
        }
    }

    /// The listening service this phone reaches with his own key, or empty.
    @Published var ownEars = UserDefaults.standard.string(forKey: "airband.ears.service") ?? ""
    /// What that service was told beyond the key.
    var ownEarsSettings: [String: String] {
        (try? JSONDecoder().decode([String: String].self, from: Data((UserDefaults.standard.string(forKey: "airband.ears.settings") ?? "{}").utf8))) ?? [:]
    }

    /// Try a listening service with a key and, if it answers, listen with it from now on. `key` nil uses the one
    /// already kept for that provider. Calls back with what is wrong, or nil.
    func keepOwnEars(_ kind: String, key: String?, settings: [String: String], done: @escaping (String?) -> Void) {
        let key = key?.trimmingCharacters(in: .whitespaces) ?? phoneKeys[kind] ?? ""
        fetch.async {
            var wrong: String?
            do { try checkOwnEars(kind: kind, key: key, settings: settings) } catch RadioError.Told(let reason) { wrong = reason } catch { wrong = "Could not check the key." }
            DispatchQueue.main.async {
                if wrong == nil {
                    if !key.isEmpty { self.keepPhoneKey(kind, key) }
                    UserDefaults.standard.set(kind, forKey: "airband.ears.service")
                    UserDefaults.standard.set(String(data: (try? JSONEncoder().encode(settings)) ?? Data("{}".utf8), encoding: .utf8), forKey: "airband.ears.settings")
                    self.ownEars = kind
                    setOwnEars(kind: kind, key: key, settings: settings)
                }
                done(wrong)
            }
        }
    }

    func forgetOwnEars() {
        UserDefaults.standard.set("", forKey: "airband.ears.service")
        ownEars = ""
        setOwnEars(kind: "", key: "", settings: [:])
    }

    /// The computers a new agent of this team can be started on, the usual one first.
    func computers(for room: String) -> [String] { desk.computersFor(room: room) }

    /// Start a new agent in a team. It appears in the list at once, with nothing to do until he speaks to it.
    /// `tool` is one of the coding tools he uses; nothing is the first of them.
    func newAgent(in room: String, on computer: String?, tool: CodingTool? = nil) {
        desk.newAgent(room: room, computer: computer, tool: tool ?? usedTools.first)
    }

    /// The coding tools he uses, as the catalogue describes them, the one he uses most first.
    var usedTools: [CodingTool] {
        let all = codingTools()
        return tools.compactMap { id in all.first { $0.id == id } }
    }

    /// Let a question go without answering it.
    func dismiss(_ m: ChatMessage) {
        if replyingTo?.id == m.id { replyingTo = nil }
        fetch.async {
            let result = Result { try self.desk.dismiss(messageId: m.id) }
            DispatchQueue.main.async {
                if case .failure(RadioError.Told(let reason)) = result { self.problem = reason }
            }
        }
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
