import ActivityKit
import SwiftUI
import WidgetKit

/// What airband puts outside its own window: the tile, which says what his agents are doing and opens straight
/// into listening (on the Home Screen, the lock screen, in StandBy, and on a car's dashboard), and the banner for
/// an agent that is waiting on him.
@main
struct AirbandWidgets: WidgetBundle {
    var body: some Widget {
        AgentsTile()
        NeedsYouBanner()
    }
}

private let ground = Colours.ground
private let ink = Colours.ink
private let ask = Colours.ask

// MARK: the tile

private struct Now: TimelineEntry {
    let date: Date
    let view: Board.View?
}

/// What the shared folder holds, once. The app and the notification extension ask for it to be read again
/// whenever what it shows changes: nothing is read on a clock.
private struct Kept: TimelineProvider {
    func placeholder(in context: Context) -> Now { Now(date: .now, view: nil) }
    func getSnapshot(in context: Context, completion: @escaping (Now) -> Void) { completion(Now(date: .now, view: Board.read())) }
    func getTimeline(in context: Context, completion: @escaping (Timeline<Now>) -> Void) { completion(Timeline(entries: [Now(date: .now, view: Board.read())], policy: .never)) }
}

private struct TileEntryView: View {
    @Environment(\.widgetFamily) private var family
    let entry: Now
    var body: some View {
        let face = TileFace(view: entry.view, family: family)
        face.containerBackground(ground, for: .widget).widgetURL(face.opens)
    }
}

struct AgentsTile: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: Board.kind, provider: Kept()) { entry in TileEntryView(entry: entry) }
            .configurationDisplayName("Agents")
            .description("Who needs you and what your agents say. Tap to talk.")
            .supportedFamilies([.systemSmall, .systemMedium, .systemLarge, .systemExtraLarge, .accessoryRectangular, .accessoryInline])
    }
}

// MARK: the banner

struct NeedsYouBanner: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: NeedsYouAttributes.self) { context in
            BannerFace(context: context)
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) { SquareBadge(text: context.attributes.badge, fill: ask, size: 36) }
                DynamicIslandExpandedRegion(.center) {
                    Text("\(context.attributes.callsign) needs you").font(.system(size: 15, weight: .semibold)).lineLimit(1)
                }
                DynamicIslandExpandedRegion(.trailing) { Image(systemName: "mic.fill").foregroundStyle(ask) }
                DynamicIslandExpandedRegion(.bottom) {
                    Text(context.state.question).font(.system(size: 13)).foregroundStyle(ink.opacity(0.8)).lineLimit(2)
                }
            } compactLeading: {
                SquareBadge(text: context.attributes.badge, fill: ask, size: 22)
            } compactTrailing: {
                Image(systemName: "mic.fill").foregroundStyle(ask)
            } minimal: {
                Image(systemName: "mic.fill").foregroundStyle(ask)
            }
            .widgetURL(OpenWith.answer(context.attributes.agent))
        }
        // the small form is what a car's dashboard (and a watch) shows
        .supplementalActivityFamilies([.small])
    }
}

/// The banner on the lock screen, and its small form on a car's dashboard: who is waiting and what it asked.
private struct BannerFace: View {
    @Environment(\.activityFamily) private var family
    let context: ActivityViewContext<NeedsYouAttributes>
    var body: some View {
        switch family {
        case .small:
            BannerSmall(badge: context.attributes.badge, callsign: context.attributes.callsign, question: context.state.question, waiting: context.state.waiting)
                .activityBackgroundTint(ground)
                .widgetURL(OpenWith.answer(context.attributes.agent))
        default:
            HStack(spacing: 12) {
                SquareBadge(text: context.attributes.badge, fill: ask, size: 40)
                VStack(alignment: .leading, spacing: 2) {
                    Text(context.state.waiting > 1 ? "\(context.attributes.callsign) needs you, and \(context.state.waiting - 1) more" : "\(context.attributes.callsign) needs you")
                        .font(.system(size: 15, weight: .bold)).foregroundStyle(ink).lineLimit(1)
                    if !context.state.question.isEmpty {
                        Text(context.state.question).font(.system(size: 14)).foregroundStyle(ink.opacity(0.8)).lineLimit(3).fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 8)
                Label("Answer", systemImage: "mic.fill").font(.system(size: 15, weight: .bold)).foregroundStyle(ground)
                    .padding(.horizontal, 12).frame(minHeight: 36).background(ink, in: Capsule())
            }
            .padding(.horizontal, 16).padding(.vertical, 12)
            .activityBackgroundTint(ground)
            .widgetURL(OpenWith.answer(context.attributes.agent))
        }
    }
}
