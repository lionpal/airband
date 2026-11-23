// The phone's own small rules (Sources/Plain.swift) and the tray's trims, on the simulator.
import XCTest
@testable import Airband

final class PlainTests: XCTestCase {
    func testNothingTypedIsSpacesAndLineBreaksAlike() {
        // a draft of line breaks is nothing to send
        XCTAssertTrue("\n\n  \n".isBlank)
        XCTAssertTrue(" \t".isBlank)
        XCTAssertFalse(" ok\n".isBlank)
        XCTAssertEqual("  key-123\n".trimmed, "key-123")
        XCTAssertNil("".orNone)
        XCTAssertEqual("a".orNone, "a")
    }

    func testAMomentInMillisecondsIsThatMoment() {
        XCTAssertEqual(Date(ms: UInt64(1_700_000_000_500)).timeIntervalSince1970, 1_700_000_000.5, accuracy: 0.0001)
        XCTAssertEqual(Date(ms: Int64(0)), Date(timeIntervalSince1970: 0))
    }

    func testTheHandlesOfATrimStayInsideAClipShorterThanASecond() {
        // a half-second clip gives no negative start and no end past its length
        let length = 0.5
        XCTAssertEqual(TrimBounds.start(0.4, end: 0.5, length: length), 0)
        XCTAssertEqual(TrimBounds.end(0.1, start: 0, length: length), 0.5)
        XCTAssertGreaterThanOrEqual(TrimBounds.start(-3, end: 0.2, length: length), 0)
        XCTAssertLessThanOrEqual(TrimBounds.end(9, start: 0.4, length: length), length)
    }

    func testTheHandlesOfATrimKeepASecondBetweenThem() {
        let length = 10.0
        XCTAssertEqual(TrimBounds.start(9.5, end: 6, length: length), 5)
        XCTAssertEqual(TrimBounds.end(2, start: 4, length: length), 5)
        XCTAssertEqual(TrimBounds.end(12, start: 4, length: length), 10)
        XCTAssertEqual(TrimBounds.start(-1, end: 6, length: length), 0)
    }

    @MainActor func testTrimmedTwiceQuicklyOnlyTheNewestClipIsSent() {
        // the first trim's export ends after the second began, and is thrown away
        let model = RadioModel.shared
        let source = FileManager.default.temporaryDirectory.appendingPathComponent("trim-\(UUID().uuidString).mov")
        FileManager.default.createFile(atPath: source.path, contents: Data())
        var item = TrayItem(video: true, number: 99)
        item.source = source
        model.tray.append(item)
        defer { model.tray.removeAll { $0.id == item.id }; try? FileManager.default.removeItem(at: source) }
        model.trim(item, to: 0...1)
        let first = model.tray.first { $0.id == item.id }!.making
        model.trim(item, to: 0...2)
        let second = model.tray.first { $0.id == item.id }!.making
        XCTAssertNotEqual(first, second)
        let late = FileManager.default.temporaryDirectory.appendingPathComponent("late-\(UUID().uuidString).mp4")
        FileManager.default.createFile(atPath: late.path, contents: Data())
        model.prepared(item.id, .success((late, "video-99.mp4", nil)), source: source, making: first)
        XCTAssertNil(model.tray.first { $0.id == item.id }!.attachment, "the older trim's clip was sent")
        XCTAssertFalse(FileManager.default.fileExists(atPath: late.path), "the older trim's clip was kept")
        XCTAssertTrue(FileManager.default.fileExists(atPath: source.path), "the video to trim again was let go")
    }

    func testAPastedLinkThatIsNotOneIsToldAsThat() {
        let board = UIPasteboard.withUniqueName()
        defer { UIPasteboard.remove(withName: board.name) }
        board.string = "  not a link\n"
        XCTAssertEqual(RadioModel.shared.pairPasted(from: board), RadioModel.notALink)
        board.string = nil
        XCTAssertNotNil(RadioModel.shared.pairPasted(from: board))
    }
}
