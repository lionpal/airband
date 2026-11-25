// Listening, end to end in the simulator: the app's own core and the app's own listening engine, a recording of
// speech given as the microphone would give it, a real time apart, and the words reaching the stand-in agent on
// the tower tools/e2e-phone.py runs. The recording is made by that script (`say`, 16 kHz 16-bit mono) and handed
// over by its path in AIRBAND_E2E_SPEECH, with the words it says in AIRBAND_E2E_SPEECH_WORDS.
import XCTest
@testable import Airband

final class ListeningEndToEndTests: XCTestCase {
    /// Fermion Phonon, fetched from the catalogue's address as the app fetches it and got ready as the app gets it
    /// ready, on the simulator's CPU: Settings says it is being got ready and then that it is ready; what he says
    /// shows as he says it; and what it heard is what the agent is given.
    func testTheListeningModelOnThePhoneHearsWhatHeSaysAndItIsSent() throws {
        let speech = try Speech()
        let tower = try Tower()
        let pairing = try tower.pair()
        let host = Speaking(speech: speech.pcm)
        let phone = Phone(host: host)
        defer { phone.unpair() }
        XCTAssertEqual(phone.desk.setLinks(links: [pairing.link]).links, [pairing.link])
        let agent = try phone.agentOnline()

        let model = try XCTUnwrap(phone.desk.choices(job: .ears).first { $0.place == .phone }, "a listening model is offered on this phone")
        // chosen: fetched, then got ready, which Settings shows while it happens
        let choosing = DispatchQueue(label: "choosing")
        var chose: Result<Void, Error>?
        let done = DispatchSemaphore(value: 0)
        choosing.async {
            chose = Result { try phone.desk.chooseFor(job: .ears, id: model.id, key: nil, settings: [:], later: false) }
            done.signal()
        }
        var sawPreparing = false
        while done.wait(timeout: .now() + 0.2) == .timedOut {
            if case .preparing? = phone.desk.choices(job: .ears).first(where: { $0.id == model.id })?.readying?.step { sawPreparing = true }
        }
        try XCTUnwrap(chose).get()
        XCTAssertTrue(sawPreparing, "Settings never said it was being got ready")
        let chosen = try XCTUnwrap(phone.desk.choices(job: .ears).first { $0.id == model.id })
        XCTAssertTrue(chosen.chosen)
        XCTAssertEqual(chosen.state, .ready)
        XCTAssertNil(phone.desk.cannotListen())

        // he talks to the agent: the microphone gives the recording as he says it, then he taps send
        phone.desk.talk(re: nil, room: nil, agent: agent)
        _ = try phone.until("the agent is given what he said", seconds: 180) { _ in
            (try? tower.heard())?.first { Speech.says($0.heard, speech.words) }?.heard
        }
        // his words showed while he was still talking
        // his words were shown before he was given what it heard: read again as he talks, here on the simulator's
        // CPU, which takes longer than the short recording does, so they may come just after he stops
        XCTAssertFalse(host.shownAtAll().isEmpty, "nothing was shown as he spoke; the model's runs: \(host.ranModel())")
        XCTAssertTrue(host.told().isEmpty, "he was told something went wrong: \(host.told())")
    }

    /// Every listening service in the catalogue, each stood in for on this Mac speaking its own protocol: chosen
    /// with a key (its key tested as the app tests it), it hears him; a live one shows his words while he is
    /// still talking; and what it heard is what the agent is given. A service with no live way of listening
    /// (one that takes the recording once he stops) shows nothing until then, and says so in its row.
    func testEveryListeningServiceHearsWhatHeSaysAndItIsSent() throws {
        let speech = try Speech()
        let tower = try Tower()
        let pairing = try tower.pair()
        let host = Speaking(speech: speech.pcm)
        let phone = Phone(host: host)
        defer { phone.unpair() }
        XCTAssertEqual(phone.desk.setLinks(links: [pairing.link]).links, [pairing.link])
        let agent = try phone.agentOnline()
        let services = phone.desk.choices(job: .ears).filter { $0.place == .service }
        XCTAssertEqual(Set(services.map(\.provider)), ["azure", "elevenlabs", "gemini", "openai", "groq", "local"])
        for service in services {
            try phone.desk.chooseFor(job: .ears, id: service.id, key: service.key ? "stand-in-key" : nil, settings: [:], later: false)
            XCTAssertNil(phone.desk.cannotListen(), service.title)
            let before = try tower.heard().filter { Speech.says($0.heard, speech.words) }.count
            // what testing its key sent it is not what he says now
            let was = try tower.listening()[service.provider]
            host.again()
            phone.desk.talk(re: nil, room: nil, agent: agent)
            _ = try phone.until("\(service.title): the agent is given what he said", seconds: 60) { _ in
                ((try? tower.heard())?.filter { Speech.says($0.heard, speech.words) }.count ?? 0) > before ? true : nil
            }
            let seen = try XCTUnwrap(tower.listening()[service.provider], "\(service.title) was never reached")
            XCTAssertEqual(seen.wrong, [], service.title)
            XCTAssertTrue(seen.final, service.title)
            XCTAssertEqual(seen.seconds - (was?.seconds ?? 0), Double(speech.pcm.count) / 32000, accuracy: 0.5, "\(service.title) was not given all of what he said")
            let live = !["groq", "local"].contains(service.provider)
            if live {
                XCTAssertGreaterThan(seen.partials - (was?.partials ?? 0), 0, service.title)
                XCTAssertFalse(host.shownWhileTalking().isEmpty, "\(service.title): nothing was shown as he spoke")
            } else {
                XCTAssertTrue(host.shownWhileTalking().isEmpty, service.title)
            }
            XCTAssertTrue(host.told().isEmpty, "\(service.title): he was told something went wrong: \(host.told())")
        }
    }
}

/// The recording the script made, and what it says.
struct Speech {
    let pcm: Data
    let words: String
    init() throws {
        let env = ProcessInfo.processInfo.environment
        let path = try XCTUnwrap(env["AIRBAND_E2E_SPEECH"], "run by tools/e2e-phone.py, which makes the recording")
        pcm = try Data(contentsOf: URL(fileURLWithPath: path))
        words = try XCTUnwrap(env["AIRBAND_E2E_SPEECH_WORDS"])
    }

    /// `said` has these words in it, one after another, whatever the punctuation, the case, or numbers written
    /// as figures.
    static func says(_ said: String, _ words: String) -> Bool {
        let (a, b) = (Speech.words(said), Speech.words(words))
        guard !b.isEmpty, a.count >= b.count else { return false }
        return (0...(a.count - b.count)).contains { Array(a[$0..<($0 + b.count)]) == b }
    }

    private static func words(_ s: String) -> [String] {
        let figures = ["1": "one", "2": "two", "3": "three", "4": "four", "5": "five"]
        return s.lowercased().map { $0.isLetter || $0.isNumber ? String($0) : " " }.joined().split(separator: " ").flatMap { w -> [String] in
            w.allSatisfy(\.isNumber) ? w.map { figures[String($0)] ?? String($0) } : [String(w)]
        }
    }
}

/// A phone's host with a microphone that says a recording, a tenth of a second each tenth of a second, then stops
/// (he tapped send), and the app's own listening engine.
final class Speaking: DeskHost, @unchecked Sendable {
    private let lock = NSCondition()
    private var left: Data
    private var started: Date?
    private var given = 0
    private var notes: [String] = []
    private var shown: [String] = []
    private var shownEver: [String] = []
    private var talking = false
    private var changes = 0
    private let speech: Data
    private var secrets: [String: String] = [:]
    let listening = NeuralEngineListening(options: NeuralEngineListening.onPhone) { folder, done, of in modelSteps(folder: folder, done: UInt32(done), of: UInt32(of)) }

    init(speech: Data) {
        self.speech = speech
        left = speech
    }

    /// He says it again, from the start.
    func again() {
        lock.lock(); defer { lock.unlock() }
        left = speech
        started = nil
        given = 0
        shown = []
        shownEver = []
        notes = []
    }

    func told() -> [String] { lock.lock(); defer { lock.unlock() }; return notes }
    /// What was shown of his words while the microphone was still giving sound.
    func shownWhileTalking() -> [String] { lock.lock(); defer { lock.unlock() }; return shown }
    /// What was shown of his words before what was heard was given back.
    func shownAtAll() -> [String] { lock.lock(); defer { lock.unlock() }; return shownEver }
    func wait(until: Date) {
        lock.lock()
        defer { lock.unlock() }
        let seen = changes
        while changes == seen && lock.wait(until: until) {}
    }

    func recordStart(rate: UInt32) -> Bool {
        lock.lock(); defer { lock.unlock() }
        talking = true
        return rate == 16000
    }
    func recordRead() -> Data {
        lock.lock()
        let begun = started ?? Date()
        started = begun
        let piece = left.prefix(3200)
        left = left.dropFirst(piece.count)
        given += 1
        talking = !piece.isEmpty
        lock.unlock()
        // as fast as he spoke it
        Thread.sleep(until: begun.addingTimeInterval(Double(given) * 0.1))
        return Data(piece)
    }
    func recordStop() { lock.lock(); talking = false; lock.unlock() }
    func hearing(text: String) {
        lock.lock(); defer { lock.unlock() }
        if talking && !text.isEmpty { shown.append(text) }
        if !text.isEmpty { shownEver.append(text) }
    }
    func runModel(engine: String, folder: String, job: ChoiceJob, input: Data) -> Data? {
        let began = Date()
        let out = engine == NeuralEngineListening.name && job == .ears ? listening.hear(folder: folder, pcm: input) : nil
        lock.lock(); runs.append("\(input.count / 32000)s of sound: \(out.map { "\($0.count) bytes" } ?? "nothing") in \(Int(Date().timeIntervalSince(began) * 1000)) ms"); lock.unlock()
        return out
    }
    /// Each run of the model: how much sound, what came back (its size only), how long it took.
    private var runs: [String] = []
    func ranModel() -> [String] { lock.lock(); defer { lock.unlock() }; return runs }
    func device() -> HostDevice {
        HostDevice(engines: [NeuralEngineListening.name], os: "ios", osVersion: UInt32(ProcessInfo.processInfo.operatingSystemVersion.majorVersion), osBuild: NeuralEngineListening.systemBuild, teamColours: UInt8(TeamColours.rgb.count))
    }
    func note(text: String) { lock.lock(); notes.append(text); changes += 1; lock.broadcast(); lock.unlock() }
    func changed() { lock.lock(); changes += 1; lock.broadcast(); lock.unlock() }
    func play(rate: UInt32, pcm: Data, last: Bool) {}
    func status(status: RadioStatus) {}
    func words(text: String, starts: [UInt32], atMs: [UInt32]) {}
    func watching() -> Bool { true }
    func secret(name: String) -> String? { lock.lock(); defer { lock.unlock() }; return secrets[name] }
    func keepSecret(name: String, value: String?) { lock.lock(); secrets[name] = value; lock.unlock() }
}
