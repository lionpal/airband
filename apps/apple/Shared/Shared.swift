// What the app and its notification extension both need: the pairing links in the Keychain, the team colours
// and the buttons a notification can carry in the folder they share. Nothing here decides anything.
import Foundation
import Security
import UIKit
import UserNotifications

/// What a Keychain read found. A read that failed (the phone still locked since it started, or any other error)
/// is never taken as "nothing kept": what is kept is then never written over.
enum KeychainRead<T> {
    case found(T)
    case none
    case failed(OSStatus)
}

/// The pairing links are kept in a Keychain group the app and its notification extension share, so the
/// extension can open a sealed notification while the app is not running. His service keys are kept in a
/// group only the app can read. Everything is kept on this phone only: never in a backup, never restored elsewhere.
enum Keychain {
    /// The shared group, from the Info.plist (the team's prefix is filled in when the app is built).
    static let group = Bundle.main.object(forInfoDictionaryKey: "AirbandKeychainGroup") as? String
    /// The app's own group: only the app has it (the extension's Info.plist does not name one).
    static let privateGroup = Bundle.main.object(forInfoDictionaryKey: "AirbandKeychainPrivate") as? String

    /// Which group an item lives in. A build that does not name the group has no place for it: nothing is
    /// read from or written to another group in its stead.
    enum Place { case shared, appOnly }
    private static func named(_ place: Place) -> String? {
        let name = place == .shared ? group : privateGroup
        return name.flatMap { $0.isEmpty ? nil : $0 }
    }

    /// Keeps `value` under `key`: the item is changed in place, so a failed write never leaves nothing kept.
    /// Returns whether it was kept.
    @discardableResult
    static func write(_ key: String, _ value: String, in place: Place) -> Bool {
        guard let name = named(place) else { return false }
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrAccount as String: key, kSecAttrAccessGroup as String: name]
        let fields: [String: Any] = [
            kSecValueData as String: Data(value.utf8),
            // readable with the screen locked, while driving; on this phone only
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let changed = SecItemUpdate(query as CFDictionary, fields as CFDictionary)
        if changed == errSecSuccess { return true }
        guard changed == errSecItemNotFound else { return false }
        return SecItemAdd(query.merging(fields) { $1 } as CFDictionary, nil) == errSecSuccess
    }
    static func read(_ key: String, in place: Place) -> KeychainRead<String> {
        guard let name = named(place) else { return .failed(errSecMissingEntitlement) }
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrAccount as String: key, kSecReturnData as String: true, kSecAttrAccessGroup as String: name]
        var out: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &out)
        if status == errSecItemNotFound { return .none }
        guard status == errSecSuccess, let data = out as? Data, let text = String(data: data, encoding: .utf8) else { return .failed(status) }
        return .found(text)
    }
    /// Every computer's pairing link.
    static func links() -> KeychainRead<[String]> { Stored.load("airband.links", in: .shared) }
    @discardableResult
    static func keepLinks(_ links: [String]) -> Bool { Stored.keep("airband.links", links, in: .shared) }
}

/// The folder the app and its extension share (the App Group's container), through the core: each team's colour
/// and the sets of buttons a notification can carry. Nothing secret is kept there. None in a build that names no
/// group, or is not signed with it: then no colours are shown and a question carries only its reply box.
enum Together {
    static let folder: SharedFolder? = {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "AirbandAppGroup") as? String, !group.isEmpty,
              let dir = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group) else { return nil }
        return SharedFolder(dir: dir.path)
    }()
    /// Each team's colour as the phone chose it, for the extension to show the same.
    static func teamColours() -> [String: UInt8] {
        folder?.colours() ?? [:]
    }
    static func keepTeamColours(_ colours: [String: UInt8]) { _ = folder?.keepColours(colours: colours) }
}

/// Something kept as JSON text, in the Keychain or in the app's own settings. The one way either is read and
/// written.
enum Stored {
    /// From the Keychain. Text that is there but does not read as `T` is a failed read, never "nothing kept".
    static func load<T: Decodable>(_ key: String, in place: Keychain.Place) -> KeychainRead<T> {
        switch Keychain.read(key, in: place) {
        case .none: return .none
        case .failed(let status): return .failed(status)
        case .found(let text):
            guard let value = try? JSONDecoder().decode(T.self, from: Data(text.utf8)) else { return .failed(errSecDecode) }
            return .found(value)
        }
    }
    @discardableResult
    static func keep<T: Encodable>(_ key: String, _ value: T, in place: Keychain.Place) -> Bool {
        guard let data = try? JSONEncoder().encode(value), let text = String(data: data, encoding: .utf8) else { return false }
        return Keychain.write(key, text, in: place)
    }
    /// From the app's own settings: what is not there, or does not read, is `empty`. Nothing secret is kept here.
    static func setting<T: Decodable>(_ key: String, or empty: T) -> T {
        UserDefaults.standard.string(forKey: key).flatMap { try? JSONDecoder().decode(T.self, from: Data($0.utf8)) } ?? empty
    }
    static func keepSetting<T: Encodable>(_ key: String, _ value: T) {
        guard let data = try? JSONEncoder().encode(value), let text = String(data: data, encoding: .utf8) else { return }
        UserDefaults.standard.set(text, forKey: key)
    }
}

/// A colour for each team, in the order the phone hands them out: blue, orange, violet, magenta, amber, green.
enum TeamColours {
    static let rgb: [(Double, Double, Double)] = [
        (0.224, 0.529, 0.898), (0.851, 0.349, 0.149), (0.565, 0.522, 0.914),
        (0.835, 0.318, 0.506), (0.788, 0.522, 0.0), (0.098, 0.620, 0.439),
    ]
    /// For a team the phone has not given a colour yet: a plain grey, no team's colour.
    static let none: (Double, Double, Double) = (0.557, 0.557, 0.576)
    static func rgb(_ place: UInt8?) -> (Double, Double, Double) { place.map { rgb[Int($0) % rgb.count] } ?? none }
    static func ui(_ place: UInt8?) -> UIColor {
        let c = rgb(place)
        return UIColor(red: c.0, green: c.1, blue: c.2, alpha: 1)
    }
}

/// The buttons on a notification. iOS wants each set of buttons registered ahead under a name (a category):
/// the fixed sets are registered when the app starts, and a question's own answers by the extension just
/// before it shows the question. A button says only which answer it is; the words are in the notification.
enum NoticeButtons {
    static let reply = "airband.reply"
    static let permit = "airband.permit"
    /// A request for leave shown by its opening: Deny and the reply box, and no Allow under part of a command.
    static let permitAfterReading = "airband.permit.read"
    static let failed = "airband.failed"
    static let typed = "reply"

    private static var type: UNTextInputNotificationAction {
        UNTextInputNotificationAction(identifier: typed, title: "Reply", options: [], textInputButtonTitle: "Send", textInputPlaceholder: "Message")
    }
    static var fixed: [UNNotificationCategory] {
        [
            UNNotificationCategory(identifier: reply, actions: [type], intentIdentifiers: [], options: []),
            // Allow lets something run on his computer: the phone must be unlocked for it (a glance, with Face ID).
            UNNotificationCategory(identifier: permit, actions: [
                UNNotificationAction(identifier: noticeAllowId(), title: "Allow", options: [.authenticationRequired]),
                UNNotificationAction(identifier: noticeDenyId(), title: "Deny", options: [.destructive]), type,
            ], intentIdentifiers: [], options: []),
            UNNotificationCategory(identifier: permitAfterReading, actions: [
                UNNotificationAction(identifier: noticeDenyId(), title: "Deny", options: [.destructive]), type,
            ], intentIdentifiers: [], options: []),
            UNNotificationCategory(identifier: failed, actions: [], intentIdentifiers: [], options: []),
        ]
    }
    /// The registered name of the set the core chose for a notification.
    static func name(for set: NoticeButtonSet, options: [String]) -> String {
        switch set {
        case .permit: return permit
        case .permitAfterReading: return permitAfterReading
        case .answers: return name(for: options)
        case .reply: return reply
        }
    }
    /// The name for a set of answers: the same answers always get the same name.
    static func name(for options: [String]) -> String { noticeAnswersName(options: options) }
    static func category(for options: [String]) -> UNNotificationCategory {
        let buttons = options.prefix(3).enumerated().map { UNNotificationAction(identifier: noticeOptionId(index: UInt32($0.offset)), title: $0.element, options: []) }
        return UNNotificationCategory(identifier: name(for: options), actions: buttons + [type], intentIdentifiers: [], options: [])
    }
    /// Register the fixed sets and the answer sets kept in the shared folder, newest first, with `adding` put
    /// first among them. The app and its extension both register, each the whole of what is kept. Nothing is
    /// locked while the system is asked: once it has them, the number of the last change kept says whether the
    /// other kept one meanwhile, and then they are registered again as they are now. Calls back with whether
    /// `adding` is registered; with no shared folder nothing of it is.
    static func register(adding options: [String]? = nil, then done: @escaping (Bool) -> Void = { _ in }) {
        guard let folder = Together.folder, let kept = folder.answerSets(adding: options) else {
            UNUserNotificationCenter.current().setNotificationCategories(Set(fixed))
            return done(options == nil)
        }
        let wanted = Set((options.map { [name(for: $0)] } ?? []) + fixed.map(\.identifier))
        set(kept, in: folder, wanted: wanted, then: done)
    }
    /// Hands the system these sets and reads them back; again, as they are now, while another change has been
    /// kept since or they are not all there.
    private static func set(_ kept: AnswerSets, in folder: SharedFolder, wanted: Set<String>, tries: Int = 3, then done: @escaping (Bool) -> Void) {
        let center = UNUserNotificationCenter.current()
        center.setNotificationCategories(Set(fixed + kept.sets.map(category(for:))))
        center.getNotificationCategories { have in
            let registered = wanted.isSubset(of: Set(have.map(\.identifier)))
            let current = folder.answerSetsRev() == kept.rev
            guard !(registered && current), tries > 1, let now = folder.answerSets(adding: nil) else { return done(registered) }
            set(now, in: folder, wanted: wanted, tries: tries - 1, then: done)
        }
    }
}

/// An agent's badge as a picture: its two characters on its team's colour. The system shows it in a circle, so
/// the colour fills the square and the characters keep to the middle.
func badgeImage(_ text: String, colour: UInt8?, side: CGFloat = 160) -> Data? {
    badgePicture(text, fill: TeamColours.ui(colour), side: side, corner: 0, letters: 0.4, scale: 1).pngData()
}
