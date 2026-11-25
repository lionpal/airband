import Foundation
import SwiftUI
import WidgetKit

/// The tile as the shared folder keeps it (the core's `tile.json`, written by the app and by the notification
/// extension). The widget has none of the Rust core in it, so the shape it reads is written out here, and a test
/// holds it to what the core writes (`TileTests`). Nothing here decides what is shown: the core did.
enum Board {
    /// The tile's kind, which the app and the extension ask the system to draw again.
    static let kind = "dev.airband.talk"
    /// The file in the shared folder.
    static let file = "tile.json"

    struct Line: Decodable, Equatable {
        let message: String
        let agent: String
        let callsign: String
        let badge: String
        let colour: UInt8?
        let text: String
    }
    struct Work: Decodable, Equatable {
        let agent: String
        let callsign: String
        let badge: String
        let colour: UInt8?
        let doing: String?
    }
    /// What the small tile shows: one kind of thing.
    enum Small: Decodable, Equatable {
        case waiting(Line, more: Int)
        case news([Line])
        case working([Work])
        case quiet
        private enum Keys: String, CodingKey { case kind, line, more, lines, agents }
        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: Keys.self)
            switch try c.decode(String.self, forKey: .kind) {
            case "waiting": self = .waiting(try c.decode(Line.self, forKey: .line), more: try c.decode(Int.self, forKey: .more))
            case "news": self = .news(try c.decode([Line].self, forKey: .lines))
            case "working": self = .working(try c.decode([Work].self, forKey: .agents))
            case "quiet": self = .quiet
            case let other: throw DecodingError.dataCorruptedError(forKey: .kind, in: c, debugDescription: "no tile of kind \(other)")
            }
        }
    }
    /// One line of a larger tile.
    enum Item: Decodable, Equatable {
        case waiting(Line)
        case news(Line)
        case working(Work)
        private enum Keys: String, CodingKey { case kind, line, work }
        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: Keys.self)
            switch try c.decode(String.self, forKey: .kind) {
            case "waiting": self = .waiting(try c.decode(Line.self, forKey: .line))
            case "news": self = .news(try c.decode(Line.self, forKey: .line))
            case "working": self = .working(try c.decode(Work.self, forKey: .work))
            case let other: throw DecodingError.dataCorruptedError(forKey: .kind, in: c, debugDescription: "no line of kind \(other)")
            }
        }
    }
    /// What each size shows.
    struct View: Decodable, Equatable {
        let small: Small
        let medium: [Item]
        let large: [Item]
        let extraLarge: [Item]
        let top: Item?
        let waiting: Int
        private enum CodingKeys: String, CodingKey { case small, medium, large, extraLarge = "extra_large", top, waiting }
    }
    /// The file as kept: only what it shows is read.
    private struct Kept: Decodable { let shows: View }

    /// What the shared folder holds now. Nothing when the build names no shared folder, nothing is kept yet, or
    /// what is kept does not read: the tile is then only the way to talk.
    static func read() -> View? {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "AirbandAppGroup") as? String, !group.isEmpty,
              let dir = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group),
              let data = try? Data(contentsOf: dir.appendingPathComponent(file)) else { return nil }
        return decode(data)
    }
    /// What a kept file shows; nothing when it does not read.
    static func decode(_ data: Data) -> View? {
        try? JSONDecoder().decode(Kept.self, from: data).shows
    }

    /// The file holds what a notification on the lock screen holds: readable once the phone has been unlocked
    /// since it started, as the notification extension needs, and never before. The core writes it whole each
    /// time, so its protection is set again after each write.
    static func protect(_ path: String) {
        try? FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: path)
    }
    /// The tile is drawn again.
    static func redraw() {
        WidgetCenter.shared.reloadTimelines(ofKind: kind)
    }
}

// MARK: drawing

private let ground = Colours.ground
private let ink = Colours.ink
private let ask = Colours.ask

private func team(_ colour: UInt8?) -> Color {
    let c = TeamColours.rgb(colour)
    return Color(red: c.0, green: c.1, blue: c.2)
}

/// Where a tap on a line goes: a waiting agent is answered; anything else opens the microphone for whoever.
func tileOpens(_ item: Board.Item?) -> URL {
    if case .waiting(let line)? = item { return OpenWith.answer(line.agent) }
    return OpenWith.talk
}
func tileOpens(_ small: Board.Small) -> URL {
    if case .waiting(let line, _) = small { return OpenWith.answer(line.agent) }
    return OpenWith.talk
}

/// Who said it: badge and callsign, the badge amber while it waits on him.
private struct Who: View {
    let badge: String
    let colour: Color
    let callsign: String
    var size: CGFloat = 20
    var trailing: String? = nil
    var body: some View {
        HStack(spacing: 6) {
            SquareBadge(text: badge, fill: colour, size: size)
            Text(callsign).font(.system(size: size * 0.7, weight: .semibold)).foregroundStyle(ink).lineLimit(1)
            if let trailing {
                Spacer(minLength: 2)
                Text(trailing).font(.system(size: size * 0.65, weight: .bold)).foregroundStyle(ask).lineLimit(1)
            }
        }
    }
}

/// Nothing is going on: the tile opens the microphone.
struct TalkFace: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Image(systemName: "mic.fill").font(.system(size: 22, weight: .semibold)).foregroundStyle(ground)
                .frame(width: 46, height: 46).background(ink, in: Circle())
            Spacer(minLength: 0)
            Text("Talk").font(.system(size: 22, weight: .bold)).foregroundStyle(ink)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
    }
}

/// The small tile: the one size a car's dashboard has. As much of the words as fits.
struct SmallFace: View {
    let small: Board.Small
    var body: some View {
        switch small {
        case .waiting(let line, let more):
            VStack(alignment: .leading, spacing: 6) {
                Who(badge: line.badge, colour: ask, callsign: line.callsign, trailing: more > 0 ? "+\(more)" : nil)
                Text(line.text).font(.system(size: 15, weight: .medium)).foregroundStyle(ink)
                    .lineLimit(5).minimumScaleFactor(0.85)
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        case .news(let lines):
            VStack(alignment: .leading, spacing: 8) {
                ForEach(lines, id: \.message) { line in
                    VStack(alignment: .leading, spacing: 3) {
                        Who(badge: line.badge, colour: team(line.colour), callsign: line.callsign, size: 18)
                        Text(line.text).font(.system(size: 13)).foregroundStyle(ink.opacity(0.9)).lineLimit(lines.count == 1 ? 5 : 2)
                    }
                }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        case .working(let agents):
            VStack(alignment: .leading, spacing: 7) {
                ForEach(agents, id: \.agent) { work in
                    VStack(alignment: .leading, spacing: 2) {
                        Who(badge: work.badge, colour: team(work.colour), callsign: work.callsign, size: 18)
                        if let doing = work.doing {
                            Text(doing).font(.system(size: 12)).foregroundStyle(ink.opacity(0.75)).lineLimit(agents.count == 1 ? 4 : 1)
                        }
                    }
                }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        case .quiet:
            TalkFace()
        }
    }
}

/// One line of a larger tile.
private struct ItemRow: View {
    let item: Board.Item
    let lines: Int
    var body: some View {
        switch item {
        case .waiting(let line):
            Link(destination: OpenWith.answer(line.agent)) { said(line, colour: ask, emphasis: true) }
        case .news(let line):
            said(line, colour: team(line.colour), emphasis: false)
        case .working(let work):
            VStack(alignment: .leading, spacing: 2) {
                Who(badge: work.badge, colour: team(work.colour), callsign: work.callsign)
                if let doing = work.doing {
                    Text(doing).font(.system(size: 13)).foregroundStyle(ink.opacity(0.75)).lineLimit(1)
                }
            }
        }
    }
    private func said(_ line: Board.Line, colour: Color, emphasis: Bool) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Who(badge: line.badge, colour: colour, callsign: line.callsign)
            Text(line.text).font(.system(size: 14, weight: emphasis ? .medium : .regular)).foregroundStyle(ink.opacity(emphasis ? 1 : 0.9)).lineLimit(lines)
        }
    }
}

/// A larger tile: more lines of every kind, most pressing first. The first is given the room to be read whole.
struct ListFace: View {
    let items: [Board.Item]
    /// How many lines of words the first may take, and each after it.
    let first: Int
    let rest: Int
    var body: some View {
        if items.isEmpty {
            TalkFace()
        } else {
            VStack(alignment: .leading, spacing: 9) {
                ForEach(Array(items.enumerated()), id: \.offset) { place, item in
                    ItemRow(item: item, lines: place == 0 ? first : rest).layoutPriority(place == 0 ? 1 : 0)
                }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }
}

/// The lock screen's line: the most pressing thing, its words after who said it.
struct AccessoryFace: View {
    let top: Board.Item?
    let inline: Bool
    var body: some View {
        switch (top, inline) {
        case (nil, _):
            Label("Talk", systemImage: "mic.fill")
        case (.waiting(let line)?, true), (.news(let line)?, true):
            Text("\(line.callsign): \(line.text)")
        case (.working(let work)?, true):
            Text(work.doing.map { "\(work.callsign): \($0)" } ?? work.callsign)
        case (.waiting(let line)?, false), (.news(let line)?, false):
            VStack(alignment: .leading, spacing: 0) {
                Text(line.callsign).font(.headline).widgetAccentable().lineLimit(1)
                Text(line.text).font(.body).lineLimit(2)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        case (.working(let work)?, false):
            VStack(alignment: .leading, spacing: 0) {
                Text(work.callsign).font(.headline).widgetAccentable().lineLimit(1)
                if let doing = work.doing { Text(doing).font(.body).lineLimit(2) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// The tile in one size, as the system asks for it.
struct TileFace: View {
    let view: Board.View?
    let family: WidgetFamily
    var body: some View {
        switch family {
        case .accessoryInline: AccessoryFace(top: view?.top, inline: true)
        case .accessoryRectangular: AccessoryFace(top: view?.top, inline: false)
        case .systemMedium: ListFace(items: view?.medium ?? [], first: 3, rest: 1)
        case .systemLarge: ListFace(items: view?.large ?? [], first: 5, rest: 2)
        case .systemExtraLarge: ListFace(items: view?.extraLarge ?? [], first: 4, rest: 3)
        default: SmallFace(small: view?.small ?? .quiet)
        }
    }
    /// Where a tap anywhere else on it goes.
    var opens: URL {
        switch family {
        case .systemSmall: return tileOpens(view?.small ?? .quiet)
        default: return tileOpens(view?.top)
        }
    }
}

/// The banner for an agent waiting on him, in its small form: what a car's dashboard shows of it. Its badge, its
/// callsign, how many more are waiting, and its question.
struct BannerSmall: View {
    let badge: String
    let callsign: String
    let question: String
    /// How many agents are waiting on him, this one included.
    let waiting: Int
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                SquareBadge(text: badge, fill: ask, size: 22)
                Text(callsign).font(.system(size: 14, weight: .bold)).foregroundStyle(ink).lineLimit(1)
                if waiting > 1 {
                    Spacer(minLength: 2)
                    Text("+\(waiting - 1)").font(.system(size: 13, weight: .bold)).foregroundStyle(ask)
                }
            }
            Text(question).font(.system(size: 14, weight: .medium)).foregroundStyle(ink).lineLimit(4)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}
