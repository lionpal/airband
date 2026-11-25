// What the core's test (crates/airband-radio/src/ears/phonon_live.rs) calls in place of the app's
// `run_model`: the app's own NeuralEngineListening, run as a phone runs it, on the processor the test names.
import CoreML
import Foundation
import PhononCoreML

nonisolated(unsafe) private var listening: NeuralEngineListening?

/// Where the model runs, as the test names it.
private enum Units: Int32 {
    case gpu = 1
    case neuralEngine = 2

    var computeUnits: MLComputeUnits {
        switch self {
        case .gpu: .cpuAndGPU
        case .neuralEngine: .cpuAndNeuralEngine
        }
    }
}

/// Set up the listening for the model in `folder`, on `units`, with the encoders a phone has: the package gives
/// iOS its functions up to 15 s, and a Mac all of them; here they are the phone's. The rest is the app's own.
/// 0 when set up; 1 for units it does not know; 2 when the model's manifest could not be read.
@_cdecl("phonon_live_open")
public func phononLiveOpen(_ units: Int32, _ folder: UnsafePointer<CChar>) -> Int32 {
    guard let units = Units(rawValue: units) else { return 1 }
    let bundle = URL(fileURLWithPath: String(cString: folder))
    guard let data = try? Data(contentsOf: bundle.appendingPathComponent("manifest.json")), let manifest = try? JSONDecoder().decode(Manifest.self, from: data) else { return 2 }
    var o = NeuralEngineListening.onPhone
    o.computeUnits = units.computeUnits
    o.functions = manifest.functions_s.filter { $0 <= 15 }
    listening = NeuralEngineListening(options: o)
    return 0
}

/// The words in `pcm` (16 kHz 16-bit mono), as the app's engine gives them to the core: UTF-8 in memory the
/// caller frees with `phonon_live_free`, its length in `len`. Null when the model could not be run.
@_cdecl("phonon_live_hear")
public func phononLiveHear(_ folder: UnsafePointer<CChar>, _ pcm: UnsafePointer<UInt8>, _ count: Int, _ len: UnsafeMutablePointer<Int>) -> UnsafeMutablePointer<UInt8>? {
    guard let listening, let words = listening.hear(folder: String(cString: folder), pcm: Data(bytes: pcm, count: count)) else { return nil }
    let out = UnsafeMutablePointer<UInt8>.allocate(capacity: max(words.count, 1))
    words.copyBytes(to: out, count: words.count)
    len.pointee = words.count
    return out
}

@_cdecl("phonon_live_free")
public func phononLiveFree(_ words: UnsafeMutablePointer<UInt8>) {
    words.deallocate()
}
