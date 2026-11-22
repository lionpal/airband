import Foundation

/// The switches a development build is launched with (`-airband.devPage chat`, for a screenshot or a script that
/// cannot tap). The one place they are read: a release build has none, and every one reads as unset.
enum Dev {
    #if DEBUG
    static func flag(_ name: String) -> Bool { UserDefaults.standard.bool(forKey: "airband." + name) }
    static func text(_ name: String) -> String? { UserDefaults.standard.string(forKey: "airband." + name) }
    static func number(_ name: String) -> Int { UserDefaults.standard.integer(forKey: "airband." + name) }
    static func list(_ name: String) -> [String]? { UserDefaults.standard.stringArray(forKey: "airband." + name) }
    #else
    static func flag(_ name: String) -> Bool { false }
    static func text(_ name: String) -> String? { nil }
    static func number(_ name: String) -> Int { 0 }
    static func list(_ name: String) -> [String]? { nil }
    #endif
}
