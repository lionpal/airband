// The phone end to end: the app's own radio core (its Desk), in the simulator, against a tower and a relay that
// tools/e2e-phone.py starts on this Mac in a throwaway folder. Run by that script, which hands the test the
// address it answers on (AIRBAND_E2E_CONTROL): it makes a fresh pairing link, stops and starts the relay, and
// says what the stand-in agent was given. Nothing here reaches a tower, relay or pairing of his.
import UIKit
import UserNotifications
import XCTest
@testable import Airband

final class PhoneEndToEndTests: XCTestCase {
    /// Paired from a fresh link, the computer is shown by its name and picture at once; what he types to the agent
    /// reaches it once, and its answer is in the chat.
    func testPairsSeesTheComputerAndHasTheAgentsAnswer() throws {
        let tower = try Tower()
        let pairing = try tower.pair()
        let phone = Phone()
        defer { phone.unpair() }
        let paired = phone.desk.setLinks(links: [pairing.link])
        XCTAssertEqual(paired.links, [pairing.link])
        XCTAssertNil(paired.unread)
        // shown from the link alone, before the computer has answered
        let shown = try XCTUnwrap(phone.desk.view().stations.first)
        XCTAssertEqual(shown.name, pairing.name)
        XCTAssertEqual(shown.shape, pairing.shape)
        XCTAssertEqual(shown.picture, pairing.picture)
        XCTAssertNotNil(UIImage(systemName: RadioModel.symbol(shown.picture)))

        let agent = try phone.agentOnline()
        let computer = try XCTUnwrap(phone.desk.view().stations.first)
        XCTAssertEqual(computer.name, pairing.name, "the name it says is the name its link gave")
        XCTAssertEqual(computer.picture, pairing.picture)

        let words = "hello from the simulator " + UUID().uuidString.prefix(8)
        XCTAssertNil(phone.desk.send(text: words, re: nil, room: nil, agent: agent), "it had somewhere to go")
        let answer = try phone.answer(to: words, from: agent, tower: tower)
        XCTAssertEqual(try tower.heard().filter { $0.heard.contains(words) }.count, 1, "the agent was given it once")
        XCTAssertTrue(phone.desk.view().feed.contains { $0.his && $0.text == words }, "what he typed is in the chat")
        XCTAssertEqual(phone.desk.view().feed.filter { !$0.his && $0.text == answer }.count, 1)
    }

    /// Two sessions he starts in a terminal while the phone is following, in a team that had nobody in it, and one
    /// of them renamed by him: both are in the phone's list within a moment, the renamed one by its new name, as
    /// the tower has them. Nothing waits for a timer: the request the phone holds open ends when they arrive.
    func testAgentsStartedMeanwhileAreInTheListAtOnce() throws {
        let tower = try Tower()
        let pairing = try tower.pair()
        let phone = Phone()
        defer { phone.unpair() }
        XCTAssertEqual(phone.desk.setLinks(links: [pairing.link]).links, [pairing.link])
        let first = try phone.agentOnline()
        let (one, two) = ("new-" + UUID().uuidString.prefix(6), "new-" + UUID().uuidString.prefix(6))
        // they end when this does: the other checks have the stand-in agent alone
        defer { for id in [one, two] { try? tower.agent(id, folder: "arcady", gone: true) } }
        try tower.agent(one, folder: "arcady")
        try tower.agent(one, folder: "arcady", title: "biking two")
        try tower.agent(two, folder: "arcady")
        let room = try phone.until("both new agents are in the list, in their own team, the renamed one by its new name", seconds: 8) { view -> DeskRoom? in
            guard let room = view.rooms.first(where: { r in r.members.contains { $0.id == one } }) else { return nil }
            let ids = Set(room.members.map(\.id))
            let renamed = room.members.first { $0.id == one }?.callsign.hasSuffix("biking two") ?? false
            return ids == [one, two] && renamed ? room : nil
        }
        XCTAssertEqual(room.agents, 2)
        XCTAssertTrue(phone.desk.view().rooms.flatMap(\.members).contains { $0.id == first }, "the one that was there stays")
    }

    /// With the relay gone what he types waits on the phone, still being tried; once the relay is back it goes,
    /// reaches the agent once, and the answer comes back.
    func testWhatHeTypesWithTheRelayGoneWaitsAndGoesWhenItIsBack() throws {
        let tower = try Tower()
        let pairing = try tower.pair()
        let phone = Phone()
        defer { phone.unpair() }
        XCTAssertEqual(phone.desk.setLinks(links: [pairing.link]).links, [pairing.link])
        let agent = try phone.agentOnline()

        XCTAssertFalse(try tower.relay("stop"), "the relay has stopped")
        let words = "said with the relay gone " + UUID().uuidString.prefix(8)
        XCTAssertNil(phone.desk.send(text: words, re: nil, room: nil, agent: agent), "it had somewhere to go")
        // tried, and still being tried: the phone says how the tries are going once they have gone on a while
        let waiting = try phone.until("it waits on the phone, being tried", seconds: 90) { view in
            view.sending.first { $0.shown == words && $0.trying != nil }
        }
        XCTAssertEqual(waiting.stage, .trying)
        XCTAssertNil(waiting.notDelivered)
        XCTAssertTrue(try tower.heard().allSatisfy { !$0.heard.contains(words) }, "the agent has not had it")

        XCTAssertTrue(try tower.relay("start"), "the relay is back")
        let answer = try phone.answer(to: words, from: agent, tower: tower)
        XCTAssertEqual(try tower.heard().filter { $0.heard.contains(words) }.count, 1, "the agent was given it once")
        XCTAssertEqual(phone.desk.view().feed.filter { !$0.his && $0.text == answer }.count, 1)
        _ = try phone.until("nothing is left waiting", seconds: 30) { view in view.sending.isEmpty ? true : nil }
    }
}

/// Notifications, as the phone shows them. tools/e2e-phone.py hands this simulator each push exactly as the relay
/// handed it to the push service; the simulator shows it as Apple's own few words, since it does not run a
/// notification extension for a push it is handed. The same payload is then opened here by the extension's own
/// code (`Opening`, which the extension runs on a phone) and shown as it would show it, with the buttons it
/// carries and how it was shown kept in the shared folder for the connection record.
final class PhonePushTests: XCTestCase {
    /// What the system presents while the app is in front, in order: what came as a push, and what was added
    /// here. The system asks on the main thread, which the test runs on: waiting runs the main loop, so it can.
    final class Shown: NSObject, UNUserNotificationCenterDelegate {
        private(set) var pushed: [UNNotificationContent] = []
        private(set) var added: [UNNotificationContent] = []
        func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification, withCompletionHandler done: @escaping (UNNotificationPresentationOptions) -> Void) {
            if notification.request.trigger is UNPushNotificationTrigger { pushed.append(notification.request.content) } else { added.append(notification.request.content) }
            done([.list])
        }
        /// Run the main loop until `fact` holds, up to `seconds`.
        func until(_ seconds: Double, _ fact: () -> Bool) -> Bool {
            let end = Date().addingTimeInterval(seconds)
            while !fact() && Date() < end {
                RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.1))
            }
            return fact()
        }
    }

    func testEachKindOfNotificationIsShownWithItsWordsAndButtons() throws {
        let tower = try Tower()
        let pairing = try tower.pair()
        // the extension finds the pairing where the app keeps it: in the Keychain group the two share
        let before = Keychain.links()
        XCTAssertTrue(Keychain.keepLinks([pairing.link]), "the pairing is kept where the extension can read it")
        defer { if case .found(let links) = before { Keychain.keepLinks(links) } else { Keychain.keepLinks([]) } }
        let folder = try XCTUnwrap(Together.folder, "the app has the folder it shares with the extension")
        let center = UNUserNotificationCenter.current()
        let shown = Shown()
        center.delegate = shown
        let allowed = expectation(description: "notifications allowed")
        // quietly, so no question about them is put up on the simulator's screen
        center.requestAuthorization(options: [.provisional, .alert, .sound]) { _, _ in allowed.fulfill() }
        wait(for: [allowed], timeout: 20)
        let registered = expectation(description: "buttons registered")
        NoticeButtons.register { _ in registered.fulfill() }
        wait(for: [registered], timeout: 20)

        // the phone is following the chat with it out of sight, as with the phone in his pocket
        let phone = Phone(front: false)
        defer { phone.unpair() }
        XCTAssertEqual(phone.desk.setLinks(links: [pairing.link]).links, [pairing.link])
        _ = try phone.agentOnline()
        let address = String(repeating: "ab", count: 32)
        phone.desk.setPush(provider: .apns, token: address)
        _ = try phone.until("the computer can wake the phone", seconds: 60) { _ in (try? tower.wakeable()) == true ? true : nil }

        let kept = folder.notices().count
        /// Ask for one kind: what the relay handed the push service for it, what the simulator showed of that
        /// push, and what the extension's code showed of it and kept.
        /// With `asking` false nothing is asked: it is the next push that is taken, as one the tower sends itself.
        func ask(_ kind: String, asking: Bool = true) throws -> (push: Tower.Push, plain: UNNotificationContent, content: UNNotificationContent, how: NoticeHow) {
            let (pushes, plains, added) = (try tower.pushes().count, shown.pushed.count, shown.added.count)
            if asking { try tower.ask(kind) }
            XCTAssertTrue(shown.until(60) { ((try? tower.pushes().count) ?? 0) > pushes }, "the tower woke the phone for the \(kind)")
            let push = try tower.pushes()[pushes]
            XCTAssertTrue(shown.until(30) { shown.pushed.count > plains }, "the simulator showed the push for the \(kind)")
            let plain = try XCTUnwrap(shown.pushed.dropFirst(plains).first)
            // as the extension is handed it: Apple's few words, and the payload as it came
            let came = UNMutableNotificationContent()
            (came.title, came.body, came.userInfo) = (push.payload.aps.alert.title, push.payload.aps.alert.body, try tower.payload(pushes))
            var opened: (UNNotificationContent, NoticeHow)?
            Opening.open(UNNotificationRequest(identifier: UUID().uuidString, content: came, trigger: nil)) { content, how in
                Opening.keep(how)
                opened = (content, how)
                center.add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
            }
            XCTAssertTrue(shown.until(30) { shown.added.count > added }, "what the extension made of the \(kind) is shown")
            let (content, how) = try XCTUnwrap(opened)
            XCTAssertEqual(folder.notices().dropFirst(kept).last?.how, how, "and how is kept for the connection record")
            XCTAssertEqual(shown.added.dropFirst(added).first?.categoryIdentifier, content.categoryIdentifier)
            return (push, plain, content, how)
        }

        let report = try ask("report")
        XCTAssertEqual([report.plain.title, report.plain.body], ["airband", "New message"], "with no extension run, Apple's own words")
        XCTAssertEqual([report.content.title, report.content.body], ["shop 1", "The checker passes on all twelve units."])
        XCTAssertEqual(report.content.categoryIdentifier, NoticeButtons.reply)
        XCTAssertEqual(report.how, .opened(message: report.content.userInfo["message"] as? String ?? "", buttons: .reply, permission: false))

        let question = try ask("question")
        XCTAssertEqual([question.plain.title, question.plain.body], ["airband", "An agent needs you"])
        XCTAssertEqual(question.content.body, "Rewrite the two easy questions, or leave them?")
        XCTAssertEqual(question.content.categoryIdentifier, NoticeButtons.reply)
        // neither holds an agent up: shown as the system usually shows one
        XCTAssertEqual([report.content.interruptionLevel, question.content.interruptionLevel], [.active, .active])

        let card = try ask("permission")
        XCTAssertEqual([card.plain.title, card.plain.body], ["airband", "An agent needs you"])
        XCTAssertEqual(card.content.body, "Wants to run `npm run deploy`. Allow?")
        XCTAssertEqual(card.content.categoryIdentifier, NoticeButtons.permit)
        XCTAssertEqual(card.content.interruptionLevel, .timeSensitive, "a request that holds an agent comes through a focus")
        guard case .opened(let message, .permit, true) = card.how else { return XCTFail("\(card.how)") }
        XCTAssertEqual(card.content.userInfo["message"] as? String, message, "an answer from it goes to the request it shows")
        let categories = try categoriesNow(center)
        XCTAssertEqual(categories[NoticeButtons.permit], [noticeAllowId(), noticeDenyId(), NoticeButtons.typed])

        let long = try ask("long-permission")
        XCTAssertEqual(long.content.categoryIdentifier, NoticeButtons.permitAfterReading, "too long to allow unread: no Allow under part of it")
        XCTAssertTrue(long.content.body.hasPrefix("Wants to run `cd /tmp/shop && echo step 1"), long.content.body)
        guard case .opened(_, .permitAfterReading, true) = long.how else { return XCTFail("\(long.how)") }
        XCTAssertEqual(categories[NoticeButtons.permitAfterReading], [noticeDenyId(), NoticeButtons.typed])

        // a request whose tool gives it 8 seconds: when they are up, a quiet notification with nothing to answer
        // takes its place, under the same id, and says how it ended
        let brief = try ask("brief-permission")
        XCTAssertEqual(brief.content.interruptionLevel, .timeSensitive)
        guard case .opened(let briefLine, .permit, true) = brief.how else { return XCTFail("\(brief.how)") }
        XCTAssertEqual(brief.push.collapse, briefLine, "kept under its line's id, for what follows to take its place")
        let over = try ask("brief-permission over", asking: false)
        XCTAssertEqual([over.plain.title, over.plain.body], ["airband", "That request is over"])
        XCTAssertEqual(over.content.body, "Wants to run `make release`. Its time ran out.")
        XCTAssertEqual(over.content.categoryIdentifier, NoticeButtons.over)
        XCTAssertEqual(over.content.interruptionLevel, .passive)
        XCTAssertNil(over.content.sound)
        XCTAssertNil(over.push.payload.aps.sound, "Apple is asked for no sound")
        XCTAssertEqual(over.push.collapse, briefLine)
        guard case .opened(briefLine, .nothing, false) = over.how else { return XCTFail("\(over.how)") }
        XCTAssertEqual(try categoriesNow(center)[NoticeButtons.over], [])

        // what reached the push service, as Apple is handed it: an alert, at once, for this app, that the
        // phone may change, carrying nothing readable but the same few words
        // read again: the simulator was handed each after the relay had its answer
        let pushes = try tower.pushes()
        XCTAssertEqual(pushes.count, 6, "one each, and nothing else")
        XCTAssertEqual(pushes.map(\.payload.aps.alert.body), [report.push, question.push, card.push, long.push, brief.push, over.push].map(\.payload.aps.alert.body))
        XCTAssertEqual(pushes.map(\.payload.aps.alert.body), ["New message", "An agent needs you", "An agent needs you", "An agent needs you", "An agent needs you", "That request is over"])
        for push in pushes {
            XCTAssertEqual([push.to, push.pushType, push.priority, push.topic], [address, "alert", "10", "dev.airband.app"])
            XCTAssertTrue(push.delivered)
            XCTAssertEqual(push.payload.aps.mutableContent, 1)
            XCTAssertEqual(push.payload.aps.alert.title, "airband")
        }

        // one no pairing here opens is still shown, in the push service's few words, and why is kept
        let unopenable = UNMutableNotificationContent()
        (unopenable.title, unopenable.body, unopenable.userInfo) = ("airband", "An agent needs you", ["sealed": "c2VhbGVkIGJ5IG5vYm9keQ=="])
        var plain: (UNNotificationContent, NoticeHow)?
        Opening.open(UNNotificationRequest(identifier: UUID().uuidString, content: unopenable, trigger: nil)) { content, how in
            Opening.keep(how)
            plain = (content, how)
        }
        let (content, how) = try XCTUnwrap(plain)
        XCTAssertEqual([content.title, content.body], ["airband", "An agent needs you"])
        XCTAssertEqual(how, .plain(why: .noPairingOpensIt))
        XCTAssertEqual(folder.notices().last?.how, .plain(why: .noPairingOpensIt))
        XCTAssertTrue(phone.desk.diagnostics(notices: folder).contains("shown without its words: none of the computers paired here sent it"))
    }

    /// Each registered set of buttons, by its name: the ids of its buttons in order.
    private func categoriesNow(_ center: UNUserNotificationCenter) throws -> [String: [String]] {
        var have: [String: [String]] = [:]
        let read = expectation(description: "categories read")
        center.getNotificationCategories { all in
            have = Dictionary(uniqueKeysWithValues: all.map { ($0.identifier, $0.actions.map(\.identifier)) })
            read.fulfill()
        }
        wait(for: [read], timeout: 20)
        return have
    }
}

/// The computer's side, as tools/e2e-phone.py serves it.
struct Tower {
    struct Pairing {
        let link: String
        let name: String
        let shape: MachineShape
        var picture: ComputerPicture { Tower.picture[shape]! }
    }
    struct Heard: Decodable {
        let heard: String
        let said: String
    }
    /// The catalogue's shapes by their names in its rows.
    static let shapes: [String: MachineShape] = ["laptop": .laptop, "mini": .mini, "studio": .studio, "imac": .imac, "pro": .pro, "": .unknown]
    /// The picture a computer he has not picked one for is drawn with: its shape's, and a plain one when its shape
    /// is not known.
    static let picture: [MachineShape: ComputerPicture] = [.laptop: .laptop, .mini: .mini, .studio: .studio, .imac: .imac, .pro: .pro, .unknown: .plain]

    let control: URL
    struct NoTower: Error, CustomStringConvertible {
        var description: String { "no AIRBAND_E2E_CONTROL: these are run by tools/e2e-phone.py, which starts the tower and relay they need" }
    }

    init() throws {
        guard let at = ProcessInfo.processInfo.environment["AIRBAND_E2E_CONTROL"], let url = URL(string: at) else {
            throw NoTower()
        }
        control = url
    }

    /// A new pairing, which the tower already serves through the relay.
    func pair() throws -> Pairing {
        struct Made: Decodable {
            let link, name, model, shape: String
            let carried: [String: String]
        }
        let made = try JSONDecoder().decode(Made.self, from: call("POST", "pair"))
        XCTAssertEqual(made.carried["name"], made.name, "the link carries the computer's name")
        XCTAssertEqual(made.carried["model"], made.model, "and its model, as its system gives it")
        XCTAssertFalse(made.model.isEmpty)
        return Pairing(link: made.link, name: made.name, shape: try XCTUnwrap(Tower.shapes[made.shape]))
    }

    /// Stop or start the relay. Whether it is listening once that is done.
    func relay(_ what: String) throws -> Bool {
        struct Done: Decodable { let listening: Bool }
        return try JSONDecoder().decode(Done.self, from: call("POST", "relay/" + what)).listening
    }

    /// An agent of his checks in, in this folder of the run's (each folder a team), with the name he gave its
    /// session when there is one.
    func agent(_ id: String, folder: String, title: String? = nil, gone: Bool = false) throws {
        var at = URLComponents()
        at.path = "agent"
        at.queryItems = [URLQueryItem(name: "id", value: id), URLQueryItem(name: "folder", value: folder)] + (title.map { [URLQueryItem(name: "title", value: $0)] } ?? []) + (gone ? [URLQueryItem(name: "gone", value: "1")] : [])
        _ = try call("POST", at.string!)
    }

    /// Have the stand-in agent report, ask, or ask for leave (`kind` as the script names them).
    func ask(_ kind: String) throws { _ = try call("POST", "ask/" + kind) }

    /// Can the tower wake the phone: its log says the phone gave it an address.
    func wakeable() throws -> Bool {
        struct Said: Decodable { let wakeable: Bool }
        return try JSONDecoder().decode(Said.self, from: call("GET", "wakeable")).wakeable
    }

    /// The payload of the push at `index`, as the phone is handed it.
    func payload(_ index: Int) throws -> [AnyHashable: Any] {
        let all = try XCTUnwrap(JSONSerialization.jsonObject(with: call("GET", "pushes")) as? [[String: Any]])
        return try XCTUnwrap(all[index]["payload"] as? [String: Any])
    }

    /// What the relay handed the push service, in order.
    struct Push: Decodable {
        struct Payload: Decodable {
            struct Aps: Decodable {
                struct Alert: Decodable { let title, body: String }
                let alert: Alert
                let mutableContent: Int
                let sound: String?
                enum CodingKeys: String, CodingKey {
                    case alert, sound
                    case mutableContent = "mutable-content"
                }
            }
            let aps: Aps
            let sealed: String
        }
        let to, pushType, priority, topic: String
        /// The id it is kept under, for a later one to take its place.
        let collapse: String?
        let payload: Payload
        let delivered: Bool
        enum CodingKeys: String, CodingKey {
            case to, priority, topic, payload, delivered, collapse
            case pushType = "push_type"
        }
    }
    func pushes() throws -> [Push] {
        try JSONDecoder().decode([Push].self, from: call("GET", "pushes"))
    }

    /// Everything the stand-in agent has been given, and what it answered each.
    func heard() throws -> [Heard] {
        try JSONDecoder().decode([Heard].self, from: call("GET", "heard"))
    }

    /// The script could not do what it was asked, and what it said.
    struct Refused: Error, CustomStringConvertible {
        let path: String
        let body: String
        var description: String { "\(path): \(body)" }
    }

    private func call(_ method: String, _ path: String) throws -> Data {
        var request = URLRequest(url: URL(string: path, relativeTo: control)!.absoluteURL, timeoutInterval: 60)
        request.httpMethod = method
        var answer: Result<Data, Error> = .failure(URLError(.timedOut))
        let done = DispatchSemaphore(value: 0)
        URLSession.shared.dataTask(with: request) { data, response, error in
            if let error { answer = .failure(error) } else if (response as? HTTPURLResponse)?.statusCode == 200, let data { answer = .success(data) } else { answer = .failure(Refused(path: path, body: String(decoding: data ?? Data(), as: UTF8.self))) }
            done.signal()
        }.resume()
        done.wait()
        return try answer.get()
    }
}

/// The phone: a Desk of its own in a throwaway folder, with a host that has no sound and keeps no keys.
final class Phone {
    let host: Host
    let desk: Desk

    /// `front`: whether the app says the chat is in front of him.
    init(front: Bool = true) {
        host = Host(front: front)
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("airband-e2e-" + UUID().uuidString)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        desk = Desk.open(dataDir: dir.path, host: host)
    }

    func unpair() { _ = desk.setLinks(links: []) }

    /// Wait until what the desk shows gives an answer, each time it says it has changed (and at least each
    /// second), up to `seconds`.
    func until<T>(_ what: String, seconds: Double, _ found: (DeskView) -> T?) throws -> T {
        let end = Date().addingTimeInterval(seconds)
        while true {
            if let it = found(desk.view()) { return it }
            if Date() >= end { break }
            host.wait(until: min(end, Date().addingTimeInterval(1)))
        }
        throw Failed(what: what, seconds: seconds, notes: host.told())
    }

    /// The computer answers, and the stand-in agent is on it: its id.
    func agentOnline() throws -> String {
        try until("the computer answers with its agent", seconds: 60) { view in
            let members = view.rooms.flatMap(\.members)
            return view.online && members.count == 1 ? members[0].id : nil
        }
    }

    /// The agent's answer to these words, once it is in the chat: what it said.
    func answer(to words: String, from agent: String, tower: Tower) throws -> String {
        try until("the agent's answer is in the chat", seconds: 120) { view in
            guard let said = (try? tower.heard())?.first(where: { $0.heard.contains(words) })?.said else { return nil }
            return view.feed.contains { !$0.his && $0.agent == agent && $0.text == said } ? said : nil
        }
    }

    struct Failed: Error, CustomStringConvertible {
        let what: String
        let seconds: Double
        let notes: [String]
        var description: String { "not within \(Int(seconds)) s: \(what). Told: \(notes)" }
    }
}

final class Host: DeskHost, @unchecked Sendable {
    private let front: Bool
    init(front: Bool) { self.front = front }
    private let lock = NSCondition()
    private var notes: [String] = []
    private var changes = 0

    func told() -> [String] { lock.lock(); defer { lock.unlock() }; return notes }
    /// Until the desk says something changed, or until then.
    func wait(until: Date) {
        lock.lock()
        defer { lock.unlock() }
        let seen = changes
        while changes == seen && lock.wait(until: until) {}
    }

    func play(rate: UInt32, pcm: Data, last: Bool) {}
    func recordStart(rate: UInt32) -> Bool { false }
    func recordRead() -> Data { Data() }
    func recordStop() {}
    func changed() { lock.lock(); changes += 1; lock.broadcast(); lock.unlock() }
    func status(status: RadioStatus) {}
    func note(text: String) { lock.lock(); notes.append(text); changes += 1; lock.broadcast(); lock.unlock() }
    func hearing(text: String) {}
    func words(text: String, starts: [UInt32], atMs: [UInt32]) {}
    func watching() -> Bool { front }
    func device() -> HostDevice {
        HostDevice(engines: [], os: "ios", osVersion: UInt32(ProcessInfo.processInfo.operatingSystemVersion.majorVersion), teamColours: UInt8(TeamColours.rgb.count))
    }
    func runModel(engine: String, folder: String, job: ChoiceJob, input: Data) -> Data? { nil }
    func secret(name: String) -> String? { nil }
    func keepSecret(name: String, value: String?) {}
}
