// What he chooses on this phone: the catalogue of services and models, each job's choices, and hearing a voice
// before choosing it.
import AVFoundation
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

    /// Play a short clip of a voice. It is fetched from where the model is kept, so it works before the model
    /// is on the phone. A second tap on the same one stops it.
    func preview(_ choice: VoiceChoice) {
        if previewing == choice.name {
            voice.stopClip()
            return
        }
        previewing = choice.name
        // fetched by the core, the one way to the network
        ask("The sample could not be fetched.", { try self.desk.voicePreview(preview: choice.preview) }) { data, wrong in
            guard self.previewing == choice.name else { return }
            guard let data else {
                self.previewing = nil
                self.problem = wrong ?? "The sample could not be played."
                return
            }
            // through `Voice`, which holds the audio session; the row stops showing it as playing when it is over
            switch self.voice.playClip(data, ended: { [weak self] in if self?.previewing == choice.name { self?.previewing = nil } }) {
            case nil: break
            case .micOpen:
                self.previewing = nil
                self.problem = "Close the microphone to hear the sample."
            case .unreadable:
                self.previewing = nil
                self.problem = "The sample could not be played."
            }
        }
    }

    /// Say a sentence in the chosen voice, so he hears what he picked.
    func sampleVoice(done: @escaping (String?) -> Void) {
        ask("The voice did not speak.", {
            let sample = try self.desk.ownVoiceSample()
            self.voice.play(rate: sample.rate, pcm: sample.pcm, last: true)
        }) { _, wrong in done(wrong) }
    }
}
