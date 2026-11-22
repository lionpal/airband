// What he chooses on this phone: the catalogue of services and models, each job's choices, and hearing a voice
// before choosing it.
import AVFoundation
import PhononCoreML
import Foundation
import SwiftUI
import UIKit

extension RadioModel {
    /// Ask for a newer catalogue, at most once every few minutes. The choosers call this when they open.
    func refreshCatalog() {
        guard Date().timeIntervalSince(catalogAsked) > 300 else { return }
        catalogAsked = Date()
        let dir = voiceDir
        fetch.async {
            if Airband.refreshCatalog(dir: dir) { DispatchQueue.main.async { self.catalogRev += 1 } }
        }
    }

    /// Ask the core again. Cheap: nothing is asked of a network.
    func refreshChoices() {
        let now = Dictionary(uniqueKeysWithValues: desk.choiceJobs().map { ($0, desk.choices(job: $0)) })
        if now != choices { choices = now }
    }

    /// The jobs he chooses something for, in the order the first run asks about them.
    var jobs: [ChoiceJob] { desk.choiceJobs() }

    /// What is chosen for a job, in a few words.
    func chosenLabel(_ job: ChoiceJob) -> String { desk.chosenLabel(job: job) }

    /// Choose something for a job. `key` is a key he has just typed; `later` only notes a model, to be fetched
    /// by `finishChoices`. The core tries a service, or fetches a model, before it takes it. Calls back with
    /// what is wrong, or nil.
    func choose(_ job: ChoiceJob, _ id: String, key: String? = nil, settings: [String: String] = [:], later: Bool = false, done: @escaping (String?) -> Void = { _ in }) {
        work(done) { try self.desk.chooseFor(job: job, id: id, key: key, settings: settings, later: later) }
    }

    /// Speak in this one of the chosen voice's voices.
    func chooseVoice(_ name: String, done: @escaping (String?) -> Void = { _ in }) {
        work(done) { try self.desk.chooseVoice(name: name) }
    }

    /// The end of the first run: fetch the models he chose that are not on this phone yet.
    func finishChoices(done: @escaping (String?) -> Void) {
        work(done) { try self.desk.finishChoices() }
    }

    /// What the core is doing about a choice right now, and how far along it is.
    func choiceWork() -> (label: String, fraction: Double?)? {
        if case .working(let label, let fraction)? = desk.choiceWork() { return (label, fraction) }
        return nil
    }

    /// Play a short clip of a voice. It is fetched from where the model is kept, so it works before the model
    /// is on the phone. A second tap on the same one stops it.
    func preview(_ choice: VoiceChoice) {
        if previewing == choice.name {
            previewPlayer?.stop()
            previewing = nil
            return
        }
        previewing = choice.name
        guard let url = URL(string: choice.preview) else { return }
        URLSession.shared.dataTask(with: url) { data, _, _ in
            DispatchQueue.main.async {
                guard self.previewing == choice.name, let data, let player = try? AVAudioPlayer(data: data) else {
                    if self.previewing == choice.name { self.previewing = nil }
                    return
                }
                try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .default, options: [.mixWithOthers])
                try? AVAudioSession.sharedInstance().setActive(true)
                self.previewPlayer = player
                player.play()
                // the row stops showing it as playing when the clip is over
                DispatchQueue.main.asyncAfter(deadline: .now() + player.duration + 0.2) { if self.previewing == choice.name { self.previewing = nil } }
            }
        }.resume()
    }

    /// Say a sentence in the chosen voice, so he hears what he picked.
    func sampleVoice(done: @escaping (String?) -> Void) {
        ask("The voice did not speak.", {
            let sample = try self.desk.ownVoiceSample()
            self.voice.play(rate: sample.rate, pcm: sample.pcm, last: true)
        }) { _, wrong in done(wrong) }
    }
}
