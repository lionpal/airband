// The notification extension. A notification arrives sealed: Apple and the relay passed it on without being
// able to read it. iOS hands it here before showing it, and `Opening` (Shared/) opens it with the Rust core and
// says what to show. If it cannot be opened, the plain form Apple was given ("airband: An agent needs you") is
// shown as it is. Either way something is shown, and how, or why it could not be opened, is kept for the
// connection record.
import UserNotifications

final class NotificationService: UNNotificationServiceExtension {
    private var deliver: ((UNNotificationContent) -> Void)?
    private var plain: UNNotificationContent?
    /// The content is handed over once: by the opening finishing, or by the time running out, which come on
    /// different threads.
    private let handing = NSLock()

    /// What hands the content over, taken so nothing else can: nothing when it has been handed already.
    private func takeDeliver() -> ((UNNotificationContent) -> Void)? {
        handing.lock()
        defer { handing.unlock() }
        let taken = deliver
        deliver = nil
        return taken
    }

    override func didReceive(_ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        handing.lock()
        (deliver, plain) = (contentHandler, request.content)
        handing.unlock()
        // held strongly until it has handed the content over: the system keeps nothing else of it meanwhile
        Opening.open(request) { content, how in
            guard let hand = self.takeDeliver() else { return }
            Opening.keep(how)
            hand(content)
        }
    }

    override func serviceExtensionTimeWillExpire() {
        guard let hand = takeDeliver(), let plain else { return }
        Opening.keep(.outOfTime)
        hand(plain)
    }
}
