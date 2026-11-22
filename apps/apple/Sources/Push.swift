// Notifications, on the app's side: asking for them once he has paired, giving each computer the phone's address,
// and sending an answer he gives on a notification. What deserves a notification is the tower's decision, and
// what it says is opened by the notification extension (Notify/); nothing is decided here.
import SwiftUI
import UIKit
import UserNotifications

final class Push: NSObject, ObservableObject, UNUserNotificationCenterDelegate {
    static let shared = Push()
    /// He said no to notifications: the pairing page offers the way to the phone's settings.
    @Published var denied = false

    override init() {
        super.init()
        // he is looking at the chat: what the notifications said is in front of him
        NotificationCenter.default.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in
            UNUserNotificationCenter.current().removeAllDeliveredNotifications()
        }
    }

    /// Called at every launch once a computer is paired, and when one is added. Asks him the first time; after
    /// that only asks Apple for the phone's address, which can change.
    func setUp() {
        #if DEBUG
        // development only: `-airband.devNoPush YES` leaves the question about notifications unasked, for a screenshot
        if Dev.flag("devNoPush") { return }
        #endif
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        NoticeButtons.register(NoticeButtons.fixed)
        center.getNotificationSettings { settings in
            switch settings.authorizationStatus {
            case .notDetermined:
                center.requestAuthorization(options: [.alert, .sound]) { granted, _ in
                    DispatchQueue.main.async {
                        self.denied = !granted
                        if granted { UIApplication.shared.registerForRemoteNotifications() }
                    }
                }
            case .denied: DispatchQueue.main.async { self.denied = true }
            default:
                DispatchQueue.main.async {
                    self.denied = false
                    UIApplication.shared.registerForRemoteNotifications()
                }
            }
        }
    }

    /// With the app in front the chat already shows it. Only "not sent" is worth a banner there.
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification, withCompletionHandler done: @escaping (UNNotificationPresentationOptions) -> Void) {
        done(notification.request.content.categoryIdentifier == NoticeButtons.failed ? [.banner, .sound] : [])
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse, withCompletionHandler done: @escaping () -> Void) {
        let info = response.notification.request.content.userInfo
        let (room, agent) = (info["room"] as? String ?? "", info["agent"] as? String ?? "")
        // what the agent is called now: the notification carries only its id
        let called = RadioModel.shared.member(agent)?.agent.callsign
        let text: String?
        var tapped: Tapped?
        switch response.actionIdentifier {
        case UNNotificationDefaultActionIdentifier:
            // a tap on the notification itself: open on that agent
            DispatchQueue.main.async {
                if !agent.isEmpty { RadioModel.shared.show(room: room, agent: agent) }
                done()
            }
            return
        case NoticeButtons.allow: (text, tapped) = (leaveWord(allow: true), .allow)
        case NoticeButtons.deny: (text, tapped) = (leaveWord(allow: false), .deny)
        case NoticeButtons.typed: text = (response as? UNTextInputNotificationResponse)?.userText
        case let id where id.hasPrefix(NoticeButtons.option):
            let options = info["options"] as? [String] ?? []
            let index = Int(id.dropFirst(NoticeButtons.option.count)).flatMap { options.indices.contains($0) ? $0 : nil }
            text = index.map { options[$0] }
            tapped = index.map { .option(index: UInt32($0)) }
        default: text = nil
        }
        guard let text = text?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty, let station = info["station"] as? String, let message = info["message"] as? String else { return done() }
        // the system allows some seconds for this with the app behind the lock screen
        let task = UIApplication.shared.beginBackgroundTask(withName: "answer", expirationHandler: nil)
        let finished = DispatchSemaphore(value: 0)
        let failure = Failure()
        DispatchQueue.global(qos: .userInitiated).async {
            do { try RadioModel.shared.answerNow(station: station, message: message, text: text, tapped: tapped) }
            catch RadioError.Told(let reason) { failure.set(reason) }
            catch RadioError.Refused { failure.set("That computer refused it.") }
            // kept on the phone and sent later, with why it did not go now; anything that will never go comes as Told
            catch RadioError.Kept(let reason) { failure.set(reason) }
            catch { failure.set("It was not sent. Open airband to send it again.") }
            finished.signal()
        }
        DispatchQueue.global(qos: .userInitiated).async {
            if finished.wait(timeout: .now() + 20) == .timedOut { failure.set("Still sending. Open airband to check.") }
            if let why = failure.get() { Self.say(title: "Not sent to " + (called ?? "your agent"), body: "“\(text)”. \(why)", room: room, agent: agent) }
            DispatchQueue.main.async {
                done()
                UIApplication.shared.endBackgroundTask(task)
            }
        }
    }

    /// Why an answer did not go, set from one thread and read from another.
    private final class Failure: @unchecked Sendable {
        private let lock = NSLock()
        private var why: String?
        func set(_ text: String) { lock.lock(); if why == nil { why = text }; lock.unlock() }
        func get() -> String? { lock.lock(); defer { lock.unlock() }; return why }
    }

    /// Tell him, where he gave the answer, that it did not go. A tap opens the app on that agent.
    private static func say(title: String, body: String, room: String, agent: String) {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        content.categoryIdentifier = NoticeButtons.failed
        content.userInfo = ["room": room, "agent": agent]
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
    }
}
