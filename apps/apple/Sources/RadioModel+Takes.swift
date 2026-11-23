// What he said aloud that did not go: the core keeps each recording and decides what is shown and what trying
// again does. This plays a recording, and passes his taps on.
import AVFoundation
import Foundation

extension RadioModel {
    /// The folder the core keeps recordings in is made here first, with the strongest protection that still
    /// lets one be written while the phone is locked (he talks on a call with the screen off): readable only
    /// while the phone is unlocked, which is when he plays or retries one. It is left out of backups, so his
    /// voice never leaves the phone that way.
    static func protectTakes(dataDir: String) {
        var folder = URL(fileURLWithPath: takesFolder(dataDir: dataDir), isDirectory: true)
        let files = FileManager.default
        do {
            try files.createDirectory(at: folder, withIntermediateDirectories: true, attributes: [.protectionKey: FileProtectionType.completeUnlessOpen])
            try files.setAttributes([.protectionKey: FileProtectionType.completeUnlessOpen], ofItemAtPath: folder.path)
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try folder.setResourceValues(values)
        } catch {
            NSLog("airband: the recordings folder could not be protected: \(error.localizedDescription)")
        }
    }

    /// Play a recording; tapped again, stop it.
    /// Played through `Voice`, which holds the audio session: never while the microphone is open.
    func playTake(_ id: String) {
        if playingTake == id {
            voice.stopClip()
            return
        }
        voice.stopClip()
        playingTake = id
        ask("That recording could not be played.", { self.desk.takeSound(id: id) }) { data, wrong in
            guard self.playingTake == id else { return }
            guard let data = data ?? nil else {
                self.playingTake = nil
                self.problem = wrong ?? "That recording could not be played."
                return
            }
            switch self.voice.playClip(data, ended: { [weak self] in if self?.playingTake == id { self?.playingTake = nil } }) {
            case nil: break
            case .micOpen:
                self.playingTake = nil
                self.problem = "Close the microphone to play your recording."
            case .unreadable:
                self.playingTake = nil
                self.problem = "That recording could not be played."
            }
        }
    }

    func takeAgain(_ id: String) { desk.takeAgain(id: id) }

    func discardTake(_ id: String) {
        if playingTake == id { voice.stopClip() }
        desk.discardTake(id: id)
    }
}
