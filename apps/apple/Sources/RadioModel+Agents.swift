// What he does with his agents and their messages: answering, sending, stopping and starting agents, teams,
// accounts, and the full text and pictures of a message.
import AVFoundation
import PhononCoreML
import Foundation
import SwiftUI
import UIKit

extension RadioModel {
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
}
