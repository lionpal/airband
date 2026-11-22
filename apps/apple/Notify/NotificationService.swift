// The notification extension. A notification arrives sealed: Apple and the relay passed it on without being
// able to read it. iOS hands it here before showing it; the Rust core opens it with the pairing's key, and this
// fills in who it is from, what it says, the agent's badge and the buttons. If it cannot be opened, the plain
// form Apple was given ("airband: New message") is shown as it is.
import Intents
import UserNotifications

final class NotificationService: UNNotificationServiceExtension {
    private var deliver: ((UNNotificationContent) -> Void)?
    private var plain: UNNotificationContent?

    override func didReceive(_ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        deliver = contentHandler
        plain = request.content
        guard let sealed = request.content.userInfo["sealed"] as? String, let note = openNotice(links: Keychain.links(), sealed: sealed, colours: Keychain.teamColours()),
              let content = request.content.mutableCopy() as? UNMutableNotificationContent else { return contentHandler(request.content) }
        content.title = note.title
        content.subtitle = note.subtitle
        content.body = note.body
        content.threadIdentifier = note.station + "/" + note.agent
        // added to what came, not in place of it: the system needs its own part of a notification left alone
        content.userInfo.merge(["station": note.station, "message": note.message, "room": note.room, "agent": note.agent, "options": note.options]) { _, mine in mine }
        content.categoryIdentifier = note.permission ? NoticeButtons.permit : note.options.isEmpty ? NoticeButtons.reply : NoticeButtons.name(for: note.options)
        let picture = badgeImage(note.badge, colour: note.colour)
        let show: () -> Void = { [weak self] in
            let shown = Self.fromAgent(content, note, picture) ?? content
            self?.deliver?(shown)
            self?.deliver = nil
        }
        // a question's own answers have to be registered as buttons before it is shown
        if note.permission || note.options.isEmpty { show() } else { NoticeButtons.register([NoticeButtons.category(for: note.options)], then: show) }
    }

    override func serviceExtensionTimeWillExpire() {
        if let plain { deliver?(plain) }
        deliver = nil
    }

    /// As a message from a person: the agent's name as the sender and its badge where a contact's picture goes.
    /// Needs Apple's Communication Notifications capability; without it this gives nothing and the name stands alone.
    private static func fromAgent(_ content: UNMutableNotificationContent, _ note: OpenedNotice, _ picture: Data?) -> UNNotificationContent? {
        guard Bundle.main.object(forInfoDictionaryKey: "AirbandCommunication") as? String == "YES", let picture else { return nil }
        let who = note.station + "/" + note.agent
        let name = note.subtitle.isEmpty ? note.title : note.title + " · " + note.subtitle
        let sender = INPerson(personHandle: INPersonHandle(value: who, type: .unknown), nameComponents: nil, displayName: name, image: INImage(imageData: picture), contactIdentifier: nil, customIdentifier: who)
        let intent = INSendMessageIntent(recipients: nil, outgoingMessageType: .outgoingMessageText, content: note.body, speakableGroupName: nil, conversationIdentifier: who, serviceName: nil, sender: sender, attachments: nil)
        let interaction = INInteraction(intent: intent, response: nil)
        interaction.direction = .incoming
        interaction.donate(completion: nil)
        guard let shown = try? content.updating(from: intent) else { return nil }
        return shown
    }
}
