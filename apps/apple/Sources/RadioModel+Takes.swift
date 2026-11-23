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
    func playTake(_ id: String) {
        if playingTake == id {
            takePlayer?.stop()
            playingTake = nil
            return
        }
        takePlayer?.stop()
        playingTake = id
        ask("That recording could not be played.", { self.desk.takeSound(id: id) }) { data, wrong in
            guard self.playingTake == id else { return }
            guard let data = data ?? nil, let player = try? AVAudioPlayer(data: data) else {
                self.playingTake = nil
                self.problem = wrong ?? "That recording could not be played."
                return
            }
            try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .default, options: [.mixWithOthers])
            try? AVAudioSession.sharedInstance().setActive(true)
            self.takePlayer = player
            self.takeEnd.ended = { [weak self] in if self?.playingTake == id { self?.playingTake = nil } }
            player.delegate = self.takeEnd
            player.play()
        }
    }

    func takeAgain(_ id: String) { desk.takeAgain(id: id) }

    func discardTake(_ id: String) {
        if playingTake == id { takePlayer?.stop(); playingTake = nil }
        desk.discardTake(id: id)
    }
}
