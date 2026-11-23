// The app's own start, and where the system hands it the address to wake the phone at.
import UIKit
import UserNotifications

final class AppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        // before the launch ends, so an answer given on a notification while the app was not running is handled
        UNUserNotificationCenter.current().delegate = Push.shared
        return true
    }
    /// The system woke the app because transfers of a model it was moving have ended: they are told to the core,
    /// and the system is told when that is done.
    func application(_ application: UIApplication, handleEventsForBackgroundURLSession identifier: String, completionHandler: @escaping () -> Void) {
        guard identifier == ModelTransfers.identifier else { return completionHandler() }
        ModelTransfers.shared.wokenFor = completionHandler
        _ = RadioModel.shared
        ModelTransfers.shared.open()
    }
    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        RadioModel.shared.pushAddress(deviceToken.map { String(format: "%02x", $0) }.joined())
    }
    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        // no address: the app works as before, without notifications. Nothing to tell him that he can act on.
    }
}
