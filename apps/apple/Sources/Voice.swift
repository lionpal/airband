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

final class Voice: NSObject, @unchecked Sendable {
    private let session = AVAudioSession.sharedInstance()
    private var keepAlive: AVAudioPlayer?
    private var tone: AVAudioPlayer?
    private enum Mode { case idle, speaking, listening }

    override init() {
        super.init()
        NotificationCenter.default.addObserver(self, selector: #selector(interrupted(_:)), name: AVAudioSession.interruptionNotification, object: nil)
    }

    /// Ask for the microphone. Returns what is missing, or nil when it is granted. The phone's own speech
    /// recognition is never used, so it is never asked for.
    func authorize() async -> String? {
        let mic = await withCheckedContinuation { c in AVAudioApplication.requestRecordPermission { c.resume(returning: $0) } }
        return mic ? nil : "airband needs the microphone to hear you. Turn it on in Settings."
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

    /// Stop whatever is playing.
    func hush() { stopPlayback() }

    func goOffDuty() {
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
            stretches.removeAll()
            clipQueued = 0
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
            // where this piece sits in the clip, when it will sound, and how fast: the clip's own clock is read
            // from these, so words shown with the voice follow the sound through a stutter or a change of pace
            stretches.append((wall: queuedUntil, clip: clipQueued, rate: Double(pace.rate)))
            clipQueued += Double(samples) / Double(rate)
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

    /// Each piece of the clip handed to the speaker: when it starts to sound, where in the clip that is (seconds),
    /// and the pace it is played at.
    private var stretches: [(wall: Date, clip: Double, rate: Double)] = []
    private var clipQueued: Double = 0

    /// How far into the clip the sound coming out of the speaker is, in seconds: from what was actually queued
    /// and when, less the time the sound takes to leave the device. Nothing when no clip is playing.
    func clipPosition(at now: Date = Date()) -> Double? {
        playLock.lock()
        defer { playLock.unlock() }
        guard playing else { return nil }
        let heard = now.addingTimeInterval(-(session.outputLatency + session.ioBufferDuration))
        guard let i = stretches.lastIndex(where: { $0.wall <= heard }) else { return 0 }
        let s = stretches[i]
        let position = s.clip + heard.timeIntervalSince(s.wall) * s.rate
        // never past what has been queued: if the next piece is late, the clock waits with the sound
        let limit = i + 1 < stretches.count ? stretches[i + 1].clip : clipQueued
        return min(position, limit)
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
    /// The microphone is being captured.
    private var recording = false
    /// He opened it himself, and the core has not come for the audio yet.
    private var held = false
    /// He has just tapped, and the microphone is on its way open.
    private var opening = false
    private var captured = Data()
    private let recLock = NSCondition()
    /// Everything that starts or stops the microphone runs here, in order, and never on the main thread: opening
    /// the audio session takes a moment, and a tap must not wait for it.
    private let micQueue = DispatchQueue(label: "dev.airband.mic", qos: .userInitiated)
    private static let micRate: Double = 16000

    /// He tapped the microphone: start capturing now. The core comes for the audio a moment later, when it has
    /// a pass for the listening service; nothing he says meanwhile is lost.
    func openMic() {
        recLock.lock(); opening = true; recLock.unlock()
        micQueue.async {
            let started = self.start()
            self.recLock.lock(); self.held = started; self.opening = false; self.recLock.unlock()
        }
    }

    /// He has the microphone open, or it is on its way open, or what he said is still waiting to be read.
    var micBusy: Bool {
        recLock.lock()
        defer { recLock.unlock() }
        return opening || recording || held
    }

    /// He tapped again: stop capturing. What was captured stays until the core has read it all.
    func closeMic() {
        micQueue.async { self.halt() }
    }

    /// The core wants the microphone at `rate`. If he has already opened it, this is that same capture, from
    /// its beginning, whether or not he has closed it again since. False if the microphone cannot be opened.
    func recordStart(rate: UInt32) -> Bool {
        micQueue.sync {
            recLock.lock()
            let mine = held
            held = false
            recLock.unlock()
            return mine ? true : start()
        }
    }

    private func start() -> Bool {
        halt()
        recLock.lock(); captured.removeAll(); recLock.unlock()
        set(.listening)
        beep(880) // "your turn"
        let input = recEngine.inputNode
        // echo cancellation and noise suppression; without it a car's cabin and the phone's own speaker get in
        try? input.setVoiceProcessingEnabled(true)
        let from = input.outputFormat(forBus: 0)
        guard from.sampleRate > 0, from.channelCount > 0,
              let to = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: Voice.micRate, channels: 1, interleaved: true),
              let converter = AVAudioConverter(from: from, to: to) else {
            NSLog("airband: no microphone input")
            set(.idle)
            return false
        }
        recLock.lock(); recording = true; recLock.unlock()
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
            // four minutes at most are held for a core that has not come for them: the oldest sound goes first
            if self.recording, self.captured.count < 8_000_000 { self.captured.append(bytes); self.recLock.signal() }
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

    /// Stop capturing, keeping what was captured for whoever is reading it.
    private func halt() {
        recLock.lock()
        let was = recording
        recording = false
        recLock.broadcast()
        recLock.unlock()
        guard was else { return }
        recEngine.inputNode.removeTap(onBus: 0)
        recEngine.stop()
        try? recEngine.inputNode.setVoiceProcessingEnabled(false) // held only while listening: it quietens playback
        beep(660) // "got it"
        set(.idle)
    }

    /// The next tenth of a second or so of the microphone. Once it has been stopped, what is left of what was
    /// captured, and then nothing: that is how the core knows he has finished.
    func recordRead() -> Data {
        recLock.lock()
        defer { recLock.unlock() }
        let deadline = Date().addingTimeInterval(0.5)
        while recording && captured.count < 3200 {
            if !recLock.wait(until: deadline) { break }
        }
        if !captured.isEmpty {
            let out = captured
            captured.removeAll(keepingCapacity: true)
            return out
        }
        // a host must never hand back nothing while still recording: that means "stopped" to the core
        return recording ? Data(count: 320) : Data()
    }

    /// The core has finished with the microphone.
    func recordStop() {
        micQueue.sync {
            halt()
            recLock.lock(); captured.removeAll(); held = false; recLock.unlock()
        }
    }

    // MARK: tones

    /// Something went wrong and he may not be looking: two short low notes, the second lower.
    func uhOh() {
        micQueue.async {
            self.beep(392)
            self.beep(294)
        }
    }

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
