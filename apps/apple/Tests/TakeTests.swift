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
}
