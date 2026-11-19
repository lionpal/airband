// The phone's half of the radio: make sound, capture sound. Every decision about when to do either, which
// service speaks or listens, and when he has finished talking is in the Rust core; this file only carries them out.
//
// Four jobs, each small:
//   play(rate, pcm, last)   audio the tower's voice made, arriving in parts, played as it arrives
//   record…                 the microphone as plain PCM, for a listening service the core talks to
//   speak(text)             Apple's own voice, when the tower is set to it or its audio is late
//   listen()                Apple's own recognition, when the tower is set to it or a service fails
//
// Microphone settings follow what Backoffice learned on real phones (apps/mobile/ios/App/App/MicAecPlugin.swift):
// voice processing on the input gives echo cancellation and noise suppression, and is only held while listening,
// because it makes everything played sound quiet.
import AVFoundation
import Speech

final class Voice: NSObject, AVSpeechSynthesizerDelegate, @unchecked Sendable {
    private let session = AVAudioSession.sharedInstance()
    private var keepAlive: AVAudioPlayer?
    private var tone: AVAudioPlayer?
    private enum Mode { case idle, speaking, listening }

    /// Seconds to wait for him to start talking, and the quiet that means he has finished (Apple's recognition only;
    /// a listening service uses the core's own rule).
    var openWindow: TimeInterval = 6
    var pause: TimeInterval = 1.6

    override init() {
        super.init()
        synth.delegate = self
        NotificationCenter.default.addObserver(self, selector: #selector(interrupted(_:)), name: AVAudioSession.interruptionNotification, object: nil)
    }

    /// Ask for the microphone and speech recognition. Returns what is missing, or nil when both are granted.
    func authorize() async -> String? {
        let mic = await withCheckedContinuation { c in AVAudioApplication.requestRecordPermission { c.resume(returning: $0) } }
        let speech = await withCheckedContinuation { c in SFSpeechRecognizer.requestAuthorization { c.resume(returning: $0 == .authorized) } }
        if !mic { return "airband needs the microphone to hear you. Turn it on in Settings." }
        if !speech { return "airband needs speech recognition to hear you. Turn it on in Settings." }
        return nil
    }

    /// On a call: hold the audio session open, quietly, so iOS keeps the app running with the screen off and other
    /// apps' music keeps playing untouched.
    func goOnDuty() {
        set(.idle)
        if keepAlive == nil {
            keepAlive = try? AVAudioPlayer(data: Voice.wav(frequency: 0, seconds: 2))
            keepAlive?.numberOfLoops = -1
            keepAlive?.volume = 0
        }
        keepAlive?.play()
    }

    /// Stop saying whatever is being said.
    func hush() { synth.stopSpeaking(at: .immediate) }

    func goOffDuty() {
        synth.stopSpeaking(at: .immediate)
        stopPlayback()
        recordStop()
        keepAlive?.stop()
        keepAlive = nil
        try? session.setActive(false, options: .notifyOthersOnDeactivation)
    }

    private func set(_ mode: Mode) {
        do {
            switch mode {
            case .idle: // mixing, not ducking: music returns to full volume
                try session.setCategory(.playback, mode: .default, options: [.mixWithOthers])
            case .speaking:
                try session.setCategory(.playback, mode: .voicePrompt, options: [.duckOthers])
            case .listening:
                try session.setCategory(.playAndRecord, mode: .voiceChat, options: [.duckOthers, .allowBluetoothHFP, .defaultToSpeaker])
            }
            try session.setActive(true)
        } catch {
            NSLog("airband: audio session \(mode): \(error.localizedDescription)")
        }
        if keepAlive?.isPlaying == false { keepAlive?.play() }
    }

    @objc private func interrupted(_ note: Notification) {
        // a phone call or Siri took the audio; when it ends, take our place back
        guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt, AVAudioSession.InterruptionType(rawValue: raw) == .ended else { return }
        if keepAlive != nil { set(.idle) }
    }

    // MARK: playing the tower's voice, as it arrives

    private let playEngine = AVAudioEngine()
    private let player = AVAudioPlayerNode()
    private let pace = AVAudioUnitTimePitch()
    private var playFormat: AVAudioFormat?
    private var playing = false
    /// When the audio queued so far will have finished playing, by the clock.
    private var queuedUntil = Date.distantPast
    private let playLock = NSLock()

    /// Queue a part of a clip. Returns at once, except on the last part, when it returns as the clip ends.
    func play(rate: UInt32, pcm: Data, last: Bool) {
        playLock.lock()
        if !playing {
            set(.speaking)
            let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: Double(rate), channels: 1, interleaved: false)!
            if playFormat != format || !playEngine.isRunning {
                playEngine.stop()
                if player.engine == nil {
                    playEngine.attach(player)
                    playEngine.attach(pace)
                }
                playEngine.connect(player, to: pace, format: format)
                playEngine.connect(pace, to: playEngine.mainMixerNode, format: format)
                playFormat = format
                do { try playEngine.start() } catch { NSLog("airband: playback did not start: \(error.localizedDescription)") }
            }
            pace.rate = 1
            player.play()
            playing = true
            queuedUntil = Date()
        }
        let samples = pcm.count / 2
        if samples > 0, let format = playFormat, let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(samples)) {
            buffer.frameLength = AVAudioFrameCount(samples)
            let out = buffer.floatChannelData![0]
            pcm.withUnsafeBytes { raw in
                let ints = raw.bindMemory(to: Int16.self)
                for i in 0..<samples { out[i] = Float(Int16(littleEndian: ints[i])) / 32768 }
            }
            // The line stuttered: what was queued ran out before this part came. Play a little faster, pitch held,
            // until the clip ends, so he is not left further and further behind.
            let now = Date()
            if queuedUntil < now.addingTimeInterval(-0.05) {
                pace.rate = 1.12
                queuedUntil = now
            }
            queuedUntil = queuedUntil.addingTimeInterval(Double(samples) / Double(rate) / Double(pace.rate))
            player.scheduleBuffer(buffer, completionHandler: nil)
        }
        let ends = queuedUntil
        playLock.unlock()
        guard last else { return }
        // wait for what is queued to be heard, then give the music back
        let wait = ends.timeIntervalSinceNow
        if wait > 0 { Thread.sleep(forTimeInterval: min(wait + 0.12, 60)) }
        stopPlayback()
        set(.idle)
    }

    private func stopPlayback() {
        playLock.lock()
        if playing {
            player.stop()
            pace.rate = 1
            playing = false
        }
        playLock.unlock()
    }

    // MARK: the microphone, as plain audio

    private let recEngine = AVAudioEngine()
    private var recording = false
    private var captured = Data()
    private let recLock = NSCondition()

    /// Start capturing 16-bit mono PCM at `rate`. False if the microphone cannot be opened.
    func recordStart(rate: UInt32) -> Bool {
        recordStop()
        set(.listening)
        beep(880) // "your turn"
        let input = recEngine.inputNode
        // echo cancellation and noise suppression; without it a car's cabin and the phone's own speaker get in
        try? input.setVoiceProcessingEnabled(true)
        let from = input.outputFormat(forBus: 0)
        guard from.sampleRate > 0, from.channelCount > 0,
              let to = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: Double(rate), channels: 1, interleaved: true),
              let converter = AVAudioConverter(from: from, to: to) else {
            NSLog("airband: no microphone input")
            set(.idle)
            return false
        }
        recLock.lock(); captured.removeAll(); recording = true; recLock.unlock()
        input.installTap(onBus: 0, bufferSize: 2048, format: from) { [weak self] buffer, _ in
            guard let self else { return }
            let capacity = AVAudioFrameCount(Double(buffer.frameLength) * to.sampleRate / from.sampleRate) + 32
            guard let out = AVAudioPCMBuffer(pcmFormat: to, frameCapacity: capacity) else { return }
            var given = false
            var error: NSError?
            converter.convert(to: out, error: &error) { _, status in
                if given { status.pointee = .noDataNow; return nil }
                given = true
                status.pointee = .haveData
                return buffer
            }
            guard error == nil, out.frameLength > 0, let data = out.int16ChannelData else { return }
            let bytes = Data(bytes: data[0], count: Int(out.frameLength) * 2)
            self.recLock.lock()
            if self.recording { self.captured.append(bytes); self.recLock.signal() }
            self.recLock.unlock()
        }
        recEngine.prepare()
        do { try recEngine.start() } catch {
            NSLog("airband: microphone did not start: \(error.localizedDescription)")
            input.removeTap(onBus: 0)
            recLock.lock(); recording = false; recLock.unlock()
            set(.idle)
            return false
        }
        return true
    }

    /// The next tenth of a second or so of the microphone. Empty once stopped.
    func recordRead() -> Data {
        recLock.lock()
        defer { recLock.unlock() }
        let deadline = Date().addingTimeInterval(0.5)
        while recording && captured.count < 3200 {
            if !recLock.wait(until: deadline) { break }
        }
        guard recording else { return Data() }
        let out = captured
        captured.removeAll(keepingCapacity: true)
        // a host must never hand back nothing while still recording: that means "stopped" to the core
        return out.isEmpty ? Data(count: 320) : out
    }

    func recordStop() {
        recLock.lock()
        let was = recording
        recording = false
        captured.removeAll()
        recLock.broadcast()
        recLock.unlock()
        guard was else { return }
        recEngine.inputNode.removeTap(onBus: 0)
        recEngine.stop()
        try? recEngine.inputNode.setVoiceProcessingEnabled(false) // held only while listening: it quietens playback
        beep(660) // "got it"
        set(.idle)
    }

    // MARK: Apple's own voice

    private let synth = AVSpeechSynthesizer()
    private var spoken: DispatchSemaphore?

    /// The best English voice installed: a Premium or Enhanced one if he has downloaded one, else the default.
    private lazy var bestVoice: AVSpeechSynthesisVoice? = {
        let english = AVSpeechSynthesisVoice.speechVoices().filter { $0.language.hasPrefix("en") }
        let ranked = english.sorted { a, b in
            if a.quality.rawValue != b.quality.rawValue { return a.quality.rawValue > b.quality.rawValue }
            return (a.language == "en-US" ? 1 : 0) > (b.language == "en-US" ? 1 : 0)
        }
        return ranked.first ?? AVSpeechSynthesisVoice(language: "en-US")
    }()

    func speak(_ text: String) {
        set(.speaking)
        let done = DispatchSemaphore(value: 0)
        spoken = done
        let u = AVSpeechUtterance(string: text)
        u.voice = bestVoice
        u.rate = AVSpeechUtteranceDefaultSpeechRate * 1.06
        synth.speak(u)
        // generous: about 2.5 words a second, plus slack; never wait forever on a callback that does not come
        _ = done.wait(timeout: .now() + 10 + Double(text.count) / 8)
        spoken = nil
        set(.idle)
    }

    func speechSynthesizer(_ s: AVSpeechSynthesizer, didFinish u: AVSpeechUtterance) { spoken?.signal() }
    func speechSynthesizer(_ s: AVSpeechSynthesizer, didCancel u: AVSpeechUtterance) { spoken?.signal() }

    // MARK: Apple's own recognition

    func listen() -> String? {
        guard let recognizer = SFSpeechRecognizer(locale: Locale(identifier: "en-US")), recognizer.isAvailable else {
            NSLog("airband: speech recognition is not available")
            return nil
        }
        set(.listening)
        defer { set(.idle) }
        beep(880) // "your turn"

        let engine = AVAudioEngine()
        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        // on the phone itself: works with no signal, and nothing he says leaves the device as audio
        if recognizer.supportsOnDeviceRecognition { request.requiresOnDeviceRecognition = true }
        let input = engine.inputNode
        try? input.setVoiceProcessingEnabled(true)
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            NSLog("airband: no microphone input")
            return nil
        }
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { buffer, _ in request.append(buffer) }
        engine.prepare()
        do { try engine.start() } catch {
            NSLog("airband: microphone did not start: \(error.localizedDescription)")
            input.removeTap(onBus: 0)
            return nil
        }

        let lock = NSLock()
        var text = ""
        var lastChange = Date()
        var finished = false
        let task = recognizer.recognitionTask(with: request) { result, error in
            lock.lock()
            defer { lock.unlock() }
            if let result {
                let heard = result.bestTranscription.formattedString
                if heard != text {
                    text = heard
                    lastChange = Date()
                }
                if result.isFinal { finished = true }
            }
            if error != nil { finished = true }
        }

        let start = Date()
        while true {
            Thread.sleep(forTimeInterval: 0.1)
            lock.lock()
            let (heard, changed, done) = (text, lastChange, finished)
            lock.unlock()
            let now = Date()
            if done { break }
            if heard.isEmpty && now.timeIntervalSince(start) > openWindow { break } // he said nothing
            if !heard.isEmpty && now.timeIntervalSince(changed) > pause { break } // he stopped
            if now.timeIntervalSince(start) > 60 { break }
        }
        engine.stop()
        input.removeTap(onBus: 0)
        try? input.setVoiceProcessingEnabled(false)
        request.endAudio()
        task.cancel()
        lock.lock()
        let heard = text.trimmingCharacters(in: .whitespacesAndNewlines)
        lock.unlock()
        if !heard.isEmpty { beep(660) } // "got it"
        return heard.isEmpty ? nil : heard
    }

    // MARK: tones

    private func beep(_ frequency: Double) {
        tone = try? AVAudioPlayer(data: Voice.wav(frequency: frequency, seconds: 0.12))
        tone?.volume = 0.5
        tone?.play()
        Thread.sleep(forTimeInterval: 0.18)
    }

    /// A mono 16-bit WAV of a sine tone (frequency 0: silence), built in memory.
    static func wav(frequency: Double, seconds: Double) -> Data {
        let rate = 22050
        let count = Int(Double(rate) * seconds)
        var samples = Data(capacity: count * 2)
        for i in 0..<count {
            // fade the ends so the tone does not click
            let edge = min(1, Double(min(i, count - i)) / (Double(rate) * 0.01))
            let v = frequency == 0 ? 0 : sin(2 * .pi * frequency * Double(i) / Double(rate)) * 0.6 * edge
            withUnsafeBytes(of: Int16(v * Double(Int16.max)).littleEndian) { samples.append(contentsOf: $0) }
        }
        var d = Data()
        func put32(_ v: Int) { withUnsafeBytes(of: UInt32(v).littleEndian) { d.append(contentsOf: $0) } }
        func put16(_ v: Int) { withUnsafeBytes(of: UInt16(v).littleEndian) { d.append(contentsOf: $0) } }
        d.append(contentsOf: Array("RIFF".utf8)); put32(36 + samples.count); d.append(contentsOf: Array("WAVEfmt ".utf8))
        put32(16); put16(1); put16(1); put32(rate); put32(rate * 2); put16(2); put16(16)
        d.append(contentsOf: Array("data".utf8)); put32(samples.count)
        d.append(samples)
        return d
    }
}
