// The tile reads what the core keeps in the shared folder with no core of its own: what the core writes is what
// the tile reads. With AIRBAND_SHOTS set (TEST_RUNNER_AIRBAND_SHOTS for xcodebuild), each state is also drawn in
// each size into that folder.
import SwiftUI
import WidgetKit
import XCTest
@testable import Airband

final class TileTests: XCTestCase {
    private func line(_ message: String, _ agent: String, _ badge: String, _ colour: UInt8?, _ text: String) -> TileLine {
        TileLine(message: message, agent: agent, callsign: agent, badge: badge, colour: colour, text: text)
    }
    private var waiting: [TileLine] {
        [line("q1", "shop two", "S2", 0, "Run the database migration on staging now, or wait until the nightly backup finishes at 2 a.m.?"),
         line("q2", "site one", "S1", 1, "Keep the old pricing page live for a week?")]
    }
    private var news: [TileLine] {
        [line("r1", "docs three", "D3", 2, "The checker passes on all twelve units. Two passages were too long and are now cut to 520 words."),
         line("r2", "ops one", "O1", 3, "Deployed 0.9.7 to the relay. Health checks are green."),
         line("r3", "shop four", "S4", 0, "Merged the cart fix. Tests pass.")]
    }
    private var working: [TileWork] {
        [TileWork(agent: "mito two", callsign: "mito two", badge: "M2", colour: 4, doing: "Lightening the molecule models"),
         TileWork(agent: "shop one", callsign: "shop one", badge: "S1", colour: 0, doing: "Running the test suite"),
         TileWork(agent: "site two", callsign: "site two", badge: "S2", colour: 1, doing: nil)]
    }
    private var states: [(String, TileFacts)] {
        [("waiting", TileFacts(waiting: waiting, news: news, working: working)),
         ("news", TileFacts(waiting: [], news: news, working: working)),
         ("working", TileFacts(waiting: [], news: [], working: working)),
         ("nothing", TileFacts(waiting: [], news: [], working: []))]
    }

    /// A folder of its own for one test.
    private func folder() throws -> SharedFolder {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("tile-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        return SharedFolder(dir: dir.path)
    }
    private func kept(_ folder: SharedFolder) throws -> Board.View {
        XCTAssertEqual(URL(fileURLWithPath: folder.tilePath()).lastPathComponent, Board.file, "the tile reads the file the core writes")
        return try XCTUnwrap(Board.decode(try Data(contentsOf: URL(fileURLWithPath: folder.tilePath()))))
    }

    func testWhatTheCoreKeepsIsWhatTheTileReads() throws {
        let shared = try folder()
        XCTAssertTrue(shared.keepTile(facts: states[0].1))
        let view = try kept(shared)
        guard case .waiting(let first, let more) = view.small else { return XCTFail("\(view.small)") }
        XCTAssertEqual([first.message, first.agent, first.text], ["q1", "shop two", waiting[0].text])
        XCTAssertEqual(more, 1)
        XCTAssertEqual(view.medium.count, 3)
        XCTAssertEqual(view.top, .waiting(Board.Line(message: "q1", agent: "shop two", callsign: "shop two", badge: "S2", colour: 0, text: waiting[0].text)))
        XCTAssertEqual(tileOpens(view.small), OpenWith.answer("shop two"))
        // the same again changes nothing to draw
        XCTAssertFalse(shared.keepTile(facts: states[0].1))
        XCTAssertTrue(shared.keepTile(facts: states[2].1))
        guard case .working(let agents) = try kept(shared).small else { return XCTFail() }
        XCTAssertEqual(agents.map(\.doing), ["Lightening the molecule models", "Running the test suite", nil])
        XCTAssertEqual(tileOpens(try kept(shared).small), OpenWith.talk)
        XCTAssertTrue(shared.keepTile(facts: states[3].1))
        XCTAssertEqual(try kept(shared).small, .quiet)
    }

    /// Each state in each size, drawn as the Home Screen draws it (on its ground) and as StandBy and a car's
    /// dashboard draw the small one (no ground, on black).
    @MainActor
    func testDrawEachStateInEachSize() throws {
        guard let out = ProcessInfo.processInfo.environment["AIRBAND_SHOTS"] else { throw XCTSkip("AIRBAND_SHOTS names no folder") }
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let sizes: [(String, WidgetFamily, CGSize)] = [
            ("small", .systemSmall, CGSize(width: 170, height: 170)), ("medium", .systemMedium, CGSize(width: 364, height: 170)),
            ("large", .systemLarge, CGSize(width: 364, height: 382)), ("extra-large", .systemExtraLarge, CGSize(width: 715, height: 330)),
            ("lock-rectangular", .accessoryRectangular, CGSize(width: 172, height: 76)), ("lock-inline", .accessoryInline, CGSize(width: 257, height: 26)),
        ]
        for (name, facts) in states {
            let shared = try folder()
            shared.keepTile(facts: facts)
            // nothing going on, on a phone that never had anything: nothing is kept, and the tile is the way to talk
            let view = (try? Data(contentsOf: URL(fileURLWithPath: shared.tilePath()))).flatMap(Board.decode)
            for (size, family, frame) in sizes {
                let accessory = family == .accessoryInline || family == .accessoryRectangular
                let face = TileFace(view: view, family: family).padding(accessory ? 4 : 16).frame(width: frame.width, height: frame.height)
                    .background(accessory ? Color.black : Colours.ground, in: RoundedRectangle(cornerRadius: accessory ? 8 : 22)).foregroundStyle(.white)
                try save(face, "\(out)/tile-\(name)-\(size).png")
            }
            // StandBy and the car's dashboard: the small tile with its ground taken away
            let car = SmallFace(small: view?.small ?? .quiet).padding(14).frame(width: 170, height: 170).background(Color.black)
            try save(car, "\(out)/tile-\(name)-small-dashboard.png")
        }
        let banner = BannerSmall(badge: "S2", callsign: "shop two", question: waiting[0].text, waiting: 2).padding(12).frame(width: 170, height: 170).background(Color.black)
        try save(banner, "\(out)/live-activity-small.png")
    }

    @MainActor
    private func save(_ view: some View, _ path: String) throws {
        let renderer = ImageRenderer(content: view.environment(\.colorScheme, .dark))
        renderer.scale = 3
        let png = try XCTUnwrap(renderer.uiImage?.pngData())
        try png.write(to: URL(fileURLWithPath: path))
    }
}
