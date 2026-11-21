import ActivityKit
import Foundation

/// An agent is waiting on him. Shown as a Live Activity, which iOS also puts on a car's screen when another
/// app is in front. It says who is waiting and what it asked, in the short form he would see in the chat.
struct NeedsYouAttributes: ActivityAttributes {
    struct ContentState: Codable, Hashable {
        /// How many agents are waiting on him now, this one included.
        var waiting: Int
        /// What this one asked, in its short form.
        var question: String
    }
    /// The agent as it is called, and the two letters that stand for it. A tap answers this agent.
    var callsign: String
    var badge: String
}

/// What his agents are up to, as three numbers. This is the banner for when the app is asleep: his computer
/// keeps it current through Apple's push service, which delivers what it is given unsealed. So it holds numbers
/// and nothing else. Who is waiting and what was asked are in the app, and in the notification for it, which
/// is sealed.
struct StatusAttributes: ActivityAttributes {
    struct ContentState: Codable, Hashable {
        /// How many agents are waiting for him.
        var waiting: Int
        /// How many are working.
        var working: Int
        /// How many reports he has not seen.
        var unread: Int
    }
    /// Which of his computers it is for: a number worked out from the pairing, the same on both sides.
    var slot: UInt32
}

/// The addresses a tile or a banner opens the app with.
enum OpenWith {
    /// Open the microphone, for whoever his words turn out to be for.
    static let talk = URL(string: "airband://talk")!
    /// Open the app as it was left.
    static let chat = URL(string: "airband://chat")!
    /// Open the microphone to answer one agent.
    static func answer(_ callsign: String) -> URL {
        var parts = URLComponents(string: "airband://answer")!
        parts.queryItems = [URLQueryItem(name: "agent", value: callsign)]
        return parts.url ?? talk
    }
    /// The agent an address of that kind names.
    static func agent(in url: URL) -> String? {
        URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.first { $0.name == "agent" }?.value
    }
}
