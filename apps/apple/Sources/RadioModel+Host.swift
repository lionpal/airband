// The app's side of the core's host: sound in and out, what changed, the keys he gave, and the models this phone
// runs itself. The core calls these on its own threads.
import AVFoundation
import PhononCoreML
import Foundation
import SwiftUI
import UIKit

extension RadioModel {
    /// The keys he has given this phone, by provider. Kept in the Keychain group only the app can read, on this
    /// phone only. Only the core asks for one, to use it: no screen is ever handed a key.
    private var keys: KeychainRead<[String: String]> {
        let here: KeychainRead<[String: String]> = Stored.load("airband.keys", in: .appOnly)
        guard case .none = here else { return here }
        // kept by an earlier build in the group the extension shares: moved to the app's own group once, and
        // taken out of the shared one only after it is safely in its new place
        let earlier: KeychainRead<[String: String]> = Stored.load("airband.keys", in: .shared)
        if case .found(let all) = earlier, Stored.keep("airband.keys", all, in: .appOnly) { Keychain.write("airband.keys", "{}", in: .shared) }
        return earlier
    }

    func secret(name: String) -> String? {
        keysLock.lock()
        defer { keysLock.unlock() }
        guard case .found(let all) = keys else { return nil }
        return all[name]
    }
    func keepSecret(name: String, value: String?) {
        keysLock.lock()
        defer { keysLock.unlock() }
        // keys that could not be read are never written over with this one alone
        var all: [String: String]
        switch keys {
        case .found(let kept): all = kept
        case .none: all = [:]
        case .failed: note(text: RadioModel.keysUnread); return
        }
        all[name] = value
        if !Stored.keep("airband.keys", all, in: .appOnly) { note(text: RadioModel.keysUnread) }
    }

    func device() -> HostDevice {
        HostDevice(engines: engines.keys.sorted(), os: "ios", osVersion: UInt32(ProcessInfo.processInfo.operatingSystemVersion.majorVersion), teamColours: UInt8(TeamColours.rgb.count))
    }

    /// The core asks for a model to be run by one of the engines above.
    func runModel(engine: String, folder: String, job: ChoiceJob, input: Data) -> Data? {
        engines[engine]?(folder, job, input)
    }

    /// Fermion's listening model on the Neural Engine: a recording (16 kHz, 16-bit, mono) turned into words.
    func phononCoreML(folder: String, job: ChoiceJob, input pcm: Data) -> Data? {
        guard job == .ears else { return nil }
        hearLock.lock()
        defer { hearLock.unlock() }
        do {
            let model = try heardWith[folder] ?? Transcriber(bundle: URL(fileURLWithPath: folder))
            heardWith[folder] = model
            let samples: [Float] = pcm.withUnsafeBytes { raw in raw.bindMemory(to: Int16.self).map { Float($0) / 32768 } }
            return Data(try model.transcribe(samples).text.utf8)
        } catch {
            NSLog("airband: the listening model failed: \(error.localizedDescription)")
            return nil
        }
    }

    /// How far the voice has got in what it is saying, in characters, by the clock of the sound itself.
    func captionReached() -> Int {
        guard let caption, let position = voice.clipPosition() else { return 0 }
        return caption.reached(position)
    }

    // MARK: DeskHost: called by the core, on its own threads

    func play(rate: UInt32, pcm: Data, last: Bool) { voice.play(rate: rate, pcm: pcm, last: last) }
    func recordStart(rate: UInt32) -> Bool { voice.recordStart(rate: rate) }
    func recordRead() -> Data { voice.recordRead() }
    func recordStop() { voice.recordStop() }
    func changed() { DispatchQueue.main.async { self.refresh() } }
    func status(status: RadioStatus) {
        DispatchQueue.main.async {
            self.status = status
            if case .speaking = status {} else { self.caption = nil }
            // the core has dealt with what he said (and not merely finished something else while he talks)
            if case .listening = status {} else if case .sending = status {} else if !self.voice.micBusy { self.dictating = false; self.micOpen = false }
            // his words stay up while they are being sent, and go when the microphone has closed and they have gone
            if case .listening = status {} else if case .sending = status {} else { self.hearing = "" }
        }
    }
    func hearing(text: String) { DispatchQueue.main.async { self.hearing = text } }
    func words(text: String, starts: [UInt32], atMs: [UInt32]) {
        DispatchQueue.main.async {
            let more = zip(starts, atMs).map { (Int($0), Double($1) / 1000) }
            if text.isEmpty { self.caption?.marks.append(contentsOf: more) } else { self.caption = Caption(text: text, marks: more) }
        }
    }
    func note(text: String) {
        // he may not be looking at the screen: it is heard as well as shown
        voice.uhOh()
        DispatchQueue.main.async { self.problem = text }
    }
    func watching() -> Bool {
        frontLock.lock()
        defer { frontLock.unlock() }
        return front
    }
}
