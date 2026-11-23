// What a banner for something he said offers, by what the core says of it.
import XCTest
@testable import Airband

final class TakeTests: XCTestCase {
    func testABannerBeingHeardAgainCanOnlyBePlayed() {
        XCTAssertFalse(TakeActions(DeskTake(id: "a", line: "Listening again.", busy: true)).canAct)
        XCTAssertTrue(TakeActions(DeskTake(id: "a", line: "Not heard. No words came back.", busy: false)).canAct)
    }

    func testTheRecordingsFolderIsLeftOutOfBackups() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("takes-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: dir) }
        RadioModel.protectTakes(dataDir: dir.path)
        let folder = URL(fileURLWithPath: takesFolder(dataDir: dir.path), isDirectory: true)
        XCTAssertEqual(try folder.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, true)
    }

    func testARecordingIsPlayedOnlyThroughTheVoiceThatHoldsTheAudio() {
        let voice = Voice()
        // what cannot be read as sound is said to be so, and nothing is played
        XCTAssertEqual(voice.playClip(Data([1, 2, 3]), ended: {}), .unreadable)
        // a clip that plays is stopped by the same voice, and says it ended
        let ended = expectation(description: "ended")
        XCTAssertNil(voice.playClip(Voice.wav(frequency: 440, seconds: 2), ended: { ended.fulfill() }))
        voice.stopClip()
        wait(for: [ended], timeout: 1)
    }
}
