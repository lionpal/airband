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
    /// The agent's id, which a tap answers; what it is called, and the two letters that stand for it.
    var agent: String
    var callsign: String
    var badge: String
}

/// The addresses a tile or a banner opens the app with.
enum OpenWith {
    /// Open the microphone, for whoever his words turn out to be for.
    static let talk = URL(string: "airband://talk")!
    /// Open the microphone to answer one agent, by its id.
    static func answer(_ agent: String) -> URL {
        var parts = URLComponents(string: "airband://answer")!
        parts.queryItems = [URLQueryItem(name: "agent", value: agent)]
        return parts.url ?? talk
    }
    /// What an address the app was opened with asks for: to talk, or to answer an agent (named, or whoever
    /// is waiting). Nothing for any other address.
    enum Route {
        case talk
        case answer(String?)
    }
    static func route(_ url: URL) -> Route? {
        guard url.scheme == talk.scheme else { return nil }
        switch url.host {
        case talk.host: return .talk
        case "answer": return .answer(agent(in: url))
        default: return nil
        }
    }
    /// The agent an address of that kind names.
    static func agent(in url: URL) -> String? {
        URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.first { $0.name == "agent" }?.value
    }
}
