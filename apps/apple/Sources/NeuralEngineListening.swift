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
        let model = try Transcriber(bundle: URL(fileURLWithPath: folder))
        // the encoders the init did not load (it loads one): a recording of any length runs on one ready now
        try model.preloadAllFunctions()
        return model
    }
}
