// What the app and its notification extension both need: the pairing links in the Keychain, the team colours,
// and the buttons a notification can carry. Nothing here decides anything.
import Foundation
import Security
import UIKit
import UserNotifications

/// The pairing links are kept in a Keychain group the app and its notification extension share, so the
/// extension can open a sealed notification while the app is not running.
enum Keychain {
    /// The shared group, from the Info.plist (the team's prefix is filled in when the app is built).
    static let group = Bundle.main.object(forInfoDictionaryKey: "AirbandKeychainGroup") as? String

    static func write(_ key: String, _ value: String) {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrAccount as String: key]
        // wherever it was: builds before notifications kept it in the app's own group, where the extension cannot look
        SecItemDelete(query as CFDictionary)
        var add = query
        add[kSecValueData as String] = Data(value.utf8)
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock // readable with the screen locked, while driving
        if let group, !group.isEmpty {
            var shared = add
            shared[kSecAttrAccessGroup as String] = group
            if SecItemAdd(shared as CFDictionary, nil) == errSecSuccess { return }
        }
        // a build with no shared group still keeps the links: the app works, notifications show their plain form
        SecItemAdd(add as CFDictionary, nil)
    }
    static func read(_ key: String) -> String? {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrAccount as String: key, kSecReturnData as String: true]
        var out: AnyObject?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }
    /// Every computer's pairing link.
    static func links() -> [String] {
        (try? JSONDecoder().decode([String].self, from: Data((read("airband.links") ?? "[]").utf8))) ?? []
    }
    /// Each team's colour as the phone chose it, kept where the notification extension can read it. Not a secret:
    /// it is here because this is the one store the app and its extension share.
    static func teamColours() -> [String: UInt8] {
        (try? JSONDecoder().decode([String: UInt8].self, from: Data((read("airband.teamColours") ?? "{}").utf8))) ?? [:]
    }
    static func keepTeamColours(_ colours: [String: UInt8]) {
        guard let data = try? JSONEncoder().encode(colours), let text = String(data: data, encoding: .utf8) else { return }
        write("airband.teamColours", text)
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
    static let failed = "airband.failed"
    static let typed = "reply", allow = "allow", deny = "deny", option = "option."

    private static var type: UNTextInputNotificationAction {
        UNTextInputNotificationAction(identifier: typed, title: "Reply", options: [], textInputButtonTitle: "Send", textInputPlaceholder: "Message")
    }
    static var fixed: [UNNotificationCategory] {
        [
            UNNotificationCategory(identifier: reply, actions: [type], intentIdentifiers: [], options: []),
            // Allow lets something run on his computer: the phone must be unlocked for it (a glance, with Face ID).
            UNNotificationCategory(identifier: permit, actions: [
                UNNotificationAction(identifier: allow, title: "Allow", options: [.authenticationRequired]),
                UNNotificationAction(identifier: deny, title: "Deny", options: [.destructive]), type,
            ], intentIdentifiers: [], options: []),
            UNNotificationCategory(identifier: failed, actions: [], intentIdentifiers: [], options: []),
        ]
    }
    /// The name for a set of answers: the same answers always get the same name.
    static func name(for options: [String]) -> String {
        var h: UInt64 = 1469598103934665603
        for b in options.joined(separator: "\u{1}").utf8 { h = (h ^ UInt64(b)) &* 1099511628211 }
        return "airband.o." + String(h, radix: 16)
    }
    static func category(for options: [String]) -> UNNotificationCategory {
        let buttons = options.prefix(3).enumerated().map { UNNotificationAction(identifier: option + String($0.offset), title: $0.element, options: []) }
        return UNNotificationCategory(identifier: name(for: options), actions: buttons + [type], intentIdentifiers: [], options: [])
    }
    /// Add these sets to the ones registered, keeping the others. Calls back once the system has them.
    static func register(_ more: [UNNotificationCategory], then done: @escaping () -> Void = {}) {
        let center = UNUserNotificationCenter.current()
        center.getNotificationCategories { have in
            let names = Set(more.map(\.identifier))
            var kept = have.filter { !names.contains($0.identifier) }
            if kept.count == have.count - names.count && names.allSatisfy({ $0.hasPrefix("airband.o.") }) { return done() } // a question asked before
            // old questions' answers are not kept for ever
            let asked = kept.filter { $0.identifier.hasPrefix("airband.o.") }
            if asked.count > 48 { kept.subtract(asked.prefix(asked.count - 48)) }
            center.setNotificationCategories(kept.union(more))
            // the system takes a moment to have them; a notification shown before that has no buttons
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.3, execute: done)
        }
    }
}

/// An agent's badge as a picture: its two characters on its team's colour. The system shows it in a circle, so
/// the colour fills the square and the characters keep to the middle.
func badgeImage(_ text: String, colour: UInt8?, side: CGFloat = 160) -> Data? {
    badgePicture(text, fill: TeamColours.ui(colour), side: side, corner: 0, letters: 0.4, scale: 1).pngData()
}
