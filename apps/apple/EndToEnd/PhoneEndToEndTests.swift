// The phone end to end: the app's own radio core (its Desk), in the simulator, against a tower and a relay that
// tools/e2e-phone.py starts on this Mac in a throwaway folder. Run by that script, which hands the test the
// address it answers on (AIRBAND_E2E_CONTROL): it makes a fresh pairing link, stops and starts the relay, and
// says what the stand-in agent was given. Nothing here reaches a tower, relay or pairing of his.
import UIKit
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
        var request = URLRequest(url: control.appendingPathComponent(path), timeoutInterval: 60)
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
    let host = Host()
    let desk: Desk

    init() {
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
    func watching() -> Bool { true }
    func device() -> HostDevice {
        HostDevice(engines: [], os: "ios", osVersion: UInt32(ProcessInfo.processInfo.operatingSystemVersion.majorVersion), teamColours: UInt8(TeamColours.rgb.count))
    }
    func runModel(engine: String, folder: String, job: ChoiceJob, input: Data) -> Data? { nil }
    func secret(name: String) -> String? { nil }
    func keepSecret(name: String, value: String?) {}
}
