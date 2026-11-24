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
        NoticeButtons.register()
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
            // a tap on the notification itself: open on that message, once the chat has caught up to it
            let (station, message, rev) = (info["station"] as? String ?? "", info["message"] as? String ?? "", (info["rev"] as? NSNumber)?.uint64Value ?? 0)
            DispatchQueue.main.async {
                if !message.isEmpty { RadioModel.shared.openMessage(station: station, message: message, rev: rev, room: room, agent: agent) }
                done()
            }
            return
        case NoticeButtons.typed: text = (response as? UNTextInputNotificationResponse)?.userText
        case let id:
            // what the button stood for, read by the core from the id it made the button with, and its words
            let button = noticeButton(action: id, options: info["options"] as? [String] ?? [])
            (text, tapped) = (button?.words, button?.tapped)
        }
        guard let text = text?.trimmed, !text.isEmpty, let station = info["station"] as? String, let message = info["message"] as? String else { return done() }
        let name = called ?? "your agent"
        // the system allows some seconds for this with the app behind the lock screen. When they run out
        // before the core has an answer, he is told it is still on its way, and the work is let go
        let finish = Once()
        var task = UIBackgroundTaskIdentifier.invalid
        let end = {
            finish.run {
                done()
                UIApplication.shared.endBackgroundTask(task)
            }
        }
        // one notice for this answer: what is said later replaces what was said before it
        let notice = "answer-" + message
        task = UIApplication.shared.beginBackgroundTask(withName: "answer") {
            Self.say(notice, title: "Still sending to " + name, body: "\u{201C}\(text)\u{201D}. Open airband to check.", room: room, agent: agent)
            end()
        }
        DispatchQueue.global(qos: .userInitiated).async {
            // what became of it, with the words he is told, comes from the core
            switch RadioModel.shared.answerNow(station: station, message: message, text: text, tapped: tapped) {
            // it went: a "still sending" said meanwhile is taken back
            case .sent: UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [notice])
            case .keptForLater(let why): Self.say(notice, title: "Waiting to send to " + name, body: "\u{201C}\(text)\u{201D}. \(why)", room: room, agent: agent)
            case .refused(let why), .notPaired(let why): Self.say(notice, title: "Not delivered to " + name, body: "\u{201C}\(text)\u{201D}. \(why)", room: room, agent: agent)
            }
            DispatchQueue.main.async { end() }
        }
    }

    /// Something done once, whichever thread asks first.
    private final class Once: @unchecked Sendable {
        private let lock = NSLock()
        private var done = false
        func run(_ work: () -> Void) {
            lock.lock()
            let first = !done
            done = true
            lock.unlock()
            if first { work() }
        }
    }

    /// Tell him, where he gave the answer, what became of it. A tap opens the app on that agent.
    /// `notice` names it: a later one of the same name takes its place.
    private static func say(_ notice: String, title: String, body: String, room: String, agent: String) {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        content.categoryIdentifier = NoticeButtons.failed
        content.userInfo = ["room": room, "agent": agent]
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: notice, content: content, trigger: nil))
    }
}
