// Getting a model chosen for this phone ready (Core ML compiling it for this phone, minutes the first time, and
// again after a system update) goes on when the app is away. The core decides what needs it and does it
// (`Desk.prepareModels`); this only asks the system for the time to do it in.
import BackgroundTasks
import UIKit

enum Preparing {
    static let identifier = "dev.airband.prepare-models"

    /// Said once, before the app finishes launching, as the system requires.
    static func register() {
        BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: nil) { task in
            let work = Thread {
                RadioModel.shared.desk.prepareModels()
                task.setTaskCompleted(success: !RadioModel.shared.desk.modelsUnprepared())
            }
            // the system wants its time back: what was not finished is asked for again, and got ready next time
            task.expirationHandler = { schedule() }
            work.start()
        }
    }

    /// The app is going away with a model not ready: a little time now to go on with it, and time later from
    /// the system, while the phone is idle, for the rest.
    static func whileAway() {
        guard RadioModel.shared.desk.modelsUnprepared() else { return }
        schedule()
        var task = UIBackgroundTaskIdentifier.invalid
        task = UIApplication.shared.beginBackgroundTask(withName: "prepare-models") {
            UIApplication.shared.endBackgroundTask(task)
        }
        Thread {
            RadioModel.shared.desk.prepareModels()
            DispatchQueue.main.async { UIApplication.shared.endBackgroundTask(task) }
        }.start()
    }

    private static func schedule() {
        let request = BGProcessingTaskRequest(identifier: identifier)
        request.requiresNetworkConnectivity = false
        request.requiresExternalPower = false
        try? BGTaskScheduler.shared.submit(request)
    }
}
