// The app's own start, and where the system hands it the address to wake the phone at.
import UIKit
import UserNotifications

final class AppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        // before the launch ends, so an answer given on a notification while the app was not running is handled
        UNUserNotificationCenter.current().delegate = Push.shared
        // the team colours an older version kept in the Keychain move to the shared folder, once
        _ = Together.teamColours()
        return true
    }
    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        RadioModel.shared.pushAddress(deviceToken.map { String(format: "%02x", $0) }.joined())
    }
    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        // no address: the app works as before, without notifications. Nothing to tell him that he can act on.
    }
}
