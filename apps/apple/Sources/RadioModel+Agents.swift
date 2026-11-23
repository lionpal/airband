// What he does with his agents and their messages: answering, sending, stopping and starting agents, teams,
// accounts, and the full text and pictures of a message.
import Foundation
import SwiftUI
import UIKit

extension RadioModel {
    /// The folders inside one of his on a computer. Calls back with them, or with what is wrong.
    func folders(on computer: String, under: String, done: @escaping (DeskFolders?, String?) -> Void) {
        ask("Couldn\u{2019}t read the folders.", { try self.desk.folders(computer: computer, under: under) }, then: done)
    }

    /// The folders the coding tools on a computer have had sessions in, newest first. Calls back with them, or
    /// with what is wrong.
    func recentFolders(on computer: String, done: @escaping ([DeskRecent]?, String?) -> Void) {
        ask("Couldn\u{2019}t read the folders.", { try self.desk.recentFolders(computer: computer) }, then: done)
    }

    /// Start an agent in a folder on a computer: in the team that works there, or a new one of this name.
    /// Calls back with the team's room id ([`DeskRoom.id`]), or with what is wrong.
    func newTeam(_ name: String, on computer: String, folder: String, make: Bool, tool: String?, done: @escaping (String?, String?) -> Void) {
        ask("The agent did not start.", { try self.desk.newTeam(computer: computer, name: name, folder: folder, make: make, tool: tool) }, then: done)
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
    /// Returns his words when nothing could take them yet, to put back where he typed them.
    @discardableResult func send(_ text: String) -> String? {
        let target = replyingTo
        replyingTo = nil
        // with nothing typed there is still something to send when files are attached: the core sends the files alone
        return desk.send(text: text, re: target?.id, room: room, agent: only)
    }

    /// Try the computers again, now.
    func reconnect() {
        DispatchQueue.global(qos: .userInitiated).async { self.desk.reconnect() }
    }

    /// Stop what an agent is doing, now. What he said to it meanwhile it then reads together.
    func stop(_ agent: DeskMember) {
        tell("It did not stop.") { try self.desk.stopAgent(agent: agent.id) }
    }

    /// End an agent its computer runs: it finishes and leaves the radio.
    func end(_ agent: DeskMember) {
        tell("It did not end.", { try self.desk.endAgent(agent: agent.id) }) { if self.only == agent.id { self.look(room: self.room) } }
    }

    /// Start a new agent in a team. It appears in the list at once, with nothing to do until he speaks to it.
    /// `tool` is one of the coding tools he uses; nothing is the one he uses when there is only one.
    func newAgent(in room: String, tool: CodingTool? = nil) {
        desk.newAgent(room: room, tool: tool ?? onlyTool)
    }

    /// The coding tools he uses, as the catalogue describes them, the one he uses most first.
    var usedTools: [CodingTool] { view.tools }
    /// The coding tool he uses, when he uses just one: then it is never asked which.
    var onlyTool: CodingTool? { view.newAgent.tool }
    /// His computer, by its id, when he has just one: then it is never asked which.
    var onlyComputer: String? { view.newAgent.computer }

    /// Let a question go without answering it.
    func dismiss(_ m: ChatMessage) {
        if replyingTo?.id == m.id { replyingTo = nil }
        tell("It was not let go.") { try self.desk.dismiss(messageId: m.id) }
    }

    /// Try again a message that was not sent.
    func sendAgain(_ o: DeskOutgoing) { desk.sendAgain(id: o.id) }
    /// Take over the agent a message did not reach, then send it there once.
    func takeOverAndSend(_ o: DeskOutgoing) {
        tell("It was not taken over.") { try self.desk.takeOverAndSend(id: o.id) }
    }
    /// Let go of a message that was not sent. Its words come back, for a screen that puts them back where he
    /// types; Delete drops them.
    @discardableResult func discardUnsent(_ o: DeskOutgoing) -> String? { desk.discardUnsent(id: o.id) }
    /// How each computer was last reached, as this phone saw it.
    func reach() -> [DeskReach] { desk.reach() }
    /// The record of failed requests, for him to copy. Made here and sent nowhere.
    func diagnostics() -> String { desk.diagnostics(notices: Together.folder) }

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
        tell("It was not sent.") { try self.desk.resend(messageId: m.id, agentId: agent.id) }
    }

    /// The phone has made a picked photo or video ready (or could not): hand it to the desk, which sends it to
    /// whoever he is looking at, and remember the id it travels under.
    /// `making` is which making-ready this was ([`TrayItem.making`]): one a newer trim has replaced is thrown away.
    @MainActor func prepared(_ item: UUID, _ made: Prepare.Made, source: URL?, making: UUID) {
        let file = try? made.get().0
        guard let i = tray.firstIndex(where: { $0.id == item }) else {
            // he took it off while it was being made
            if let file { try? FileManager.default.removeItem(at: file) }
            if let source { try? FileManager.default.removeItem(at: source) }
            return
        }
        guard tray[i].making == making else {
            // trimmed again meanwhile: the newer clip is the one that goes, and the source stays for it
            if let file { try? FileManager.default.removeItem(at: file) }
            return
        }
        tray[i].source = source
        let result = made
        let made: (URL, String, UIImage?)
        switch result {
        case .success(let got): made = got
        case .failure(let unmade):
            tray[i].why = unmade.why
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

    /// Fetch what the agent actually wrote, once it arrives. Until then the opening is shown; a fetch that
    /// failed is tried again the next time it is on screen.
    func loadFull(_ m: ChatMessage) {
        guard m.long, full[m.id] == nil, !fetching.contains(m.id) else { return }
        fetching.insert(m.id)
        fetch.async {
            let text = try? self.desk.full(messageId: m.id)
            DispatchQueue.main.async {
                self.fetching.remove(m.id)
                if let text { self.full[m.id] = text }
            }
        }
    }

    /// Read one message aloud, by its id: who said it, its short form, and the answers it offers. Whatever
    /// was being read is left off.
    func readAloud(_ message: String) {
        stopReading()
        reading = message
        DispatchQueue.global(qos: .userInitiated).async {
            _ = self.desk.readQuestion(messageId: message)
            DispatchQueue.main.async { if self.reading == message { self.reading = nil } }
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
        switching = Switching(station: station, account: a.id)
        accountsProblem = nil
        ask("It did not switch.", { try self.desk.useAccount(station: station, harness: a.harness, id: a.id) }) { now, wrong in
            self.switching = nil
            self.accountsProblem = wrong
            if let now, let i = self.accounts.firstIndex(where: { $0.station == station }) { self.accounts[i] = now }
        }
    }

    /// Rename a team, by its room. Its room stays the same, so whoever was looking at it still is.
    func renameTeam(_ room: DeskRoom, to name: String) {
        // what the computer will call it, by the core's rule: the one place a team's name is cleaned
        let name = teamName(written: name)
        guard !name.isEmpty, name != room.name else { return }
        ask("It was not renamed.", { try self.desk.renameTeam(room: room.id, to: name) }) { _, wrong in
            if let wrong { self.problem = wrong }
        }
    }

    /// Put an agent in another team: a room on screen, or a new team of the name he typed.
    func moveAgent(_ agent: DeskMember, to pick: DeskTeamPick) {
        if case .new(let name) = pick, teamName(written: name).isEmpty { return }
        tell("It did not move.", { try self.desk.moveAgent(agent: agent.id, to: pick) }) {}
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

    /// Turn the call on or off. Turning it on needs the microphone: when it is not allowed the call stays off,
    /// and he is told on the phone unless `tell` is false (the car tells him itself).
    @MainActor func setCalling(_ on: Bool, tell: Bool = true) async {
        if on {
            if let missing = await voice.authorize() {
                if tell { problem = missing }
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
        await talk(re: message?.id)
    }
    /// The same, about a message known by its id: what he says answers it.
    @MainActor func talk(re message: String?) async {
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
        let target = message ?? replyingTo?.id
        replyingTo = nil
        // who it is for is worked out by the core, as for anything he types
        desk.talk(re: target, room: room, agent: only)
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
}
