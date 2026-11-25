// The listening model this app runs itself, on the Neural Engine: a recording (16 kHz, 16-bit, mono) turned into
// words. The core decides when it runs and what comes of it; this only runs it.
import Foundation
import PhononCoreML

final class NeuralEngineListening: @unchecked Sendable {
    /// The engine's name, as a model's row in the catalogue gives it (`engine`). The row and this name are one contract.
    static let name = "phonon-coreml"

    /// The build of the system this runs on ("23A341"): Core ML's compiled models are kept for one build, and
    /// an update can drop them.
    static let systemBuild: String = {
        var size = 0
        sysctlbyname("kern.osversion", nil, &size, nil, 0)
        var build = [CChar](repeating: 0, count: max(size, 1))
        sysctlbyname("kern.osversion", &build, &size, nil, 0)
        return String(cString: build)
    }()

    /// How every model is run: on which of the phone's processors, with which of its encoders.
    private let options: Transcriber.Options

    /// The phone's way: on the Neural Engine, with the encoders the package gives a phone (up to 15 s each; a
    /// longer recording is heard in windows of them). `tools/phonon-live` runs this same file on a Mac with the
    /// phone's encoders stated, on the GPU or the Neural Engine.
    static var onPhone: Transcriber.Options {
        var o = Transcriber.Options()
        o.computeUnits = .cpuAndNeuralEngine
        // One window up to just under 15 s. The package's own rule for a phone takes up to 15.0 s in one, and
        // a recording of 14.98 to 15 s then needs more frames than its 15 s encoder holds: it fails
        // (PhononCoreML error 4, met by tools/phonon-live.sh as his words passed 15 s). A longer recording is
        // cut into windows that keep inside it, by the package's rule for those: 15 s less 400 samples.
        o.singleShotMaxSeconds = 15 - 400.0 / 16000
        return o
    }

    /// Told, as a model is got ready, how many of its encoders are loaded (by the folder it is in): once it is
    /// known how many there are, then as each is done. The app tells the core; a tool on a Mac, nobody.
    private let steps: (String, Int, Int) -> Void

    init(options: Transcriber.Options, steps: @escaping (_ folder: String, _ done: Int, _ of: Int) -> Void) {
        self.options = options
        self.steps = steps
    }

    /// One model, run one thing at a time.
    private final class Slot {
        let lock = NSLock()
        var model: Transcriber?
    }
    /// By the folder each model is in, each with a lock of its own.
    private var slots: [String: Slot] = [:]
    private let lock = NSLock()

    /// The words in `pcm`, or nothing when the model could not be loaded or run (why is logged, by its code; never
    /// his words or his sound). The first call for a folder loads the model with every encoder it has: Core ML
    /// compiles each for this phone the first time on each build of the system, a minute or more apiece, which
    /// the core has done when it gets the model ready, and never while he is talking. After that a load is a
    /// moment.
    func hear(folder: String, pcm: Data) -> Data? {
        let slot = self.slot(folder)
        slot.lock.lock()
        defer { slot.lock.unlock() }
        do {
            let model = try slot.model ?? load(folder)
            slot.model = model
            let samples: [Float] = pcm.withUnsafeBytes { raw in raw.bindMemory(to: Int16.self).map { Float($0) / 32768 } }
            return Data(try model.transcribe(samples).text.utf8)
        } catch {
            let e = error as NSError
            NSLog("airband: the listening model failed: \(e.domain) \(e.code)")
            return nil
        }
    }

    private func slot(_ folder: String) -> Slot {
        lock.lock()
        defer { lock.unlock() }
        if let slot = slots[folder] { return slot }
        let slot = Slot()
        slots[folder] = slot
        return slot
    }

    private func load(_ folder: String) throws -> Transcriber {
        // no encoder is loaded by the init: each is loaded below, one at a time, so the core can be told as each
        // is done (the first time, Core ML compiles each for this phone's Neural Engine, minutes apiece)
        var options = self.options
        options.eagerFunctions = []
        let model = try Transcriber(bundle: URL(fileURLWithPath: folder), options: options)
        let encoders = model.functionSeconds
        steps(folder, 0, encoders.count)
        // An encoder is loaded by its first run, which also takes a second or two more than any after it
        // (tools/phonon-live.sh measured 1.1 to 1.8 s on a Mac's Neural Engine, once for each: his first words,
        // and again as he passed 5 and 10 s). Each is run once here, while the model is got ready or loaded at
        // launch, on a quiet tone just shorter than it, which the smallest encoder that holds it runs (silence
        // is skipped without running anything).
        for (i, seconds) in encoders.enumerated() {
            let tone = (0..<Int((seconds - 0.5) * 16000)).map { Float(sin(Double($0) * 0.07)) * 0.01 }
            _ = try model.transcribe(tone)
            steps(folder, i + 1, encoders.count)
        }
        // any encoder no tone above landed on: a recording of any length runs on one ready now
        try model.preloadAllFunctions()
        return model
    }
}
