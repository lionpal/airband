import Foundation

/// How the screens call the core: off the main thread, and back on it with what came of it in the core's words.
extension RadioModel {
    /// What went wrong, in words for him: the core's words for any of its errors, and otherwise `fallback`,
    /// which says what did not happen.
    static func words(for error: Error, else fallback: String) -> String {
        (error as? RadioError).map { errorWords(error: $0) } ?? fallback
    }

    /// Run one of the core's blocking calls off the main thread and come back on it: with what the call gave,
    /// or with what went wrong in words. Every call that can fail goes through here.
    func ask<T>(_ fallback: String, _ call: @escaping () throws -> T, then: @escaping (T?, String?) -> Void) {
        fetch.async {
            let result = Result { try call() }
            DispatchQueue.main.async {
                switch result {
                case .success(let got): then(got, nil)
                case .failure(let error): then(nil, Self.words(for: error, else: fallback))
                }
            }
        }
    }

    /// The same, for something done to an agent or a message: what went wrong is shown to him, and `done`
    /// runs when nothing did.
    func tell(_ fallback: String, _ call: @escaping () throws -> Void, done: @escaping () -> Void = {}) {
        ask(fallback, call) { _, wrong in
            if let wrong { self.problem = wrong } else { done() }
        }
    }

    /// One of the core's calls that changes what is chosen. Calls back with what it said was wrong.
    func work(_ done: @escaping (String?) -> Void, _ call: @escaping () throws -> Void) {
        ask("That did not work. Try again.", call) { _, wrong in
            self.keepModelsOutOfBackup()
            self.refreshChoices()
            done(wrong)
        }
    }

    /// A model can be fetched again, so the folder they are kept in is left out of the phone's backups.
    private func keepModelsOutOfBackup() {
        var folder = URL(fileURLWithPath: voiceDir).appendingPathComponent("models")
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? folder.setResourceValues(values)
    }
}
