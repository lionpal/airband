// What the notification extension does with a notification, in one place the app's tests can run too: open it
// with the pairing that sealed it (the Rust core does), fill in who it is from, what it says, the agent's
// badge and the buttons, and say how it is to be shown, for the shared folder's record (`keep`). What cannot be
// opened is shown as the push service delivered it, in the same few words for every notification, and why is
// said beside it. Something is always shown.
import Intents
import UserNotifications

enum Opening {
    /// Open `request` and hand over what to show, once, with how it is shown.
    static func open(_ request: UNNotificationRequest, show hand: @escaping (UNNotificationContent, NoticeHow) -> Void) {
        let sealed = request.content.userInfo["sealed"] as? String
        let opening: NoticeOpening
        switch Keychain.links() {
        case .failed(let status): opening = .plain(why: .keysUnreadable(status: status))
        case .none: opening = openNotice(links: [], sealed: sealed, colours: [:])
        case .found(let links): opening = openNotice(links: links, sealed: sealed, colours: Together.teamColours())
        }
        let note: OpenedNotice
        switch opening {
        case .plain(let why): return hand(request.content, .plain(why: why))
        case .opened(let opened): note = opened
        }
        // a notification's content always copies as one that can be changed
        let content = request.content.mutableCopy() as! UNMutableNotificationContent
        content.title = note.title
        content.subtitle = note.subtitle
        content.body = note.body
        content.threadIdentifier = note.thread
        // added to what came, not in place of it: the system needs its own part of a notification left alone
        content.userInfo.merge(["station": note.station, "message": note.message, "room": note.room, "agent": note.agent, "options": note.options]) { _, mine in mine }
        content.categoryIdentifier = NoticeButtons.name(for: note.buttons, options: note.options)
        let picture = badgeImage(note.badge, colour: note.colour)
        let opened = NoticeHow.opened(message: note.message, buttons: note.buttons, permission: note.permission)
        // a question's own answers have to be registered as buttons before it is shown. Where they could not
        // be, it carries the reply box alone: no button there stands for an answer it does not give
        guard note.buttons == .answers else { return hand(fromAgent(content, note, picture) ?? content, opened) }
        NoticeButtons.register(adding: note.options) { registered in
            if !registered { content.categoryIdentifier = NoticeButtons.reply }
            hand(fromAgent(content, note, picture) ?? content, registered ? opened : .answersNotRegistered(message: note.message))
        }
    }

    /// How a notification was shown, kept for the connection record once it has been handed to the system.
    static func keep(_ how: NoticeHow) {
        _ = Together.folder?.keepNotice(shown: NoticeShown(at: UInt64(Date().timeIntervalSince1970 * 1000), how: how))
    }

    /// As a message from a person: the agent's name as the sender and its badge where a contact's picture goes.
    /// Needs Apple's Communication Notifications capability; without it this gives nothing and the name stands alone.
    private static func fromAgent(_ content: UNMutableNotificationContent, _ note: OpenedNotice, _ picture: Data?) -> UNNotificationContent? {
        guard Bundle.main.object(forInfoDictionaryKey: "AirbandCommunication") as? String == "YES", let picture else { return nil }
        let who = note.thread
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
