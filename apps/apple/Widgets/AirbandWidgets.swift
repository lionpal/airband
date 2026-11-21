import ActivityKit
import SwiftUI
import WidgetKit

/// What airband puts outside its own window: a small tile that opens straight into listening (on the Home
/// Screen, and on a car's dashboard), and the banner for an agent that is waiting on him.
@main
struct AirbandWidgets: WidgetBundle {
    var body: some Widget {
        TalkTile()
        NeedsYouBanner()
    }
}

private let ground = Color(red: 0.043, green: 0.043, blue: 0.047)
private let ink = Color(red: 0.957, green: 0.957, blue: 0.941)
private let ask = Color(red: 0.898, green: 0.725, blue: 0.306)

// MARK: the tile

private struct Now: TimelineEntry { let date: Date }

private struct Always: TimelineProvider {
    func placeholder(in context: Context) -> Now { Now(date: .now) }
    func getSnapshot(in context: Context, completion: @escaping (Now) -> Void) { completion(Now(date: .now)) }
    func getTimeline(in context: Context, completion: @escaping (Timeline<Now>) -> Void) { completion(Timeline(entries: [Now(date: .now)], policy: .never)) }
}

struct TalkTile: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "dev.airband.talk", provider: Always()) { _ in
            VStack(alignment: .leading, spacing: 10) {
                Image(systemName: "mic.fill").font(.system(size: 22, weight: .semibold)).foregroundStyle(ground)
                    .frame(width: 46, height: 46).background(ink, in: Circle())
                Spacer(minLength: 0)
                Text("Talk").font(.system(size: 22, weight: .bold)).foregroundStyle(ink)
                Text("to your agents").font(.system(size: 14)).foregroundStyle(ink.opacity(0.6))
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
            .containerBackground(ground, for: .widget)
            .widgetURL(OpenWith.talk)
        }
        .configurationDisplayName("Talk")
        .description("Opens airband listening.")
        .supportedFamilies([.systemSmall])
    }
}

// MARK: the banner

private struct AgentBadge: View {
    let text: String
    var size: CGFloat = 36
    var body: some View {
        Text(text).font(.system(size: size * 0.42, weight: .bold, design: .rounded)).foregroundStyle(ground)
            .frame(width: size, height: size).background(ask, in: RoundedRectangle(cornerRadius: size * 0.28))
    }
}

struct NeedsYouBanner: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: NeedsYouAttributes.self) { context in
            HStack(spacing: 12) {
                AgentBadge(text: context.attributes.badge, size: 40)
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
            .widgetURL(OpenWith.answer(context.attributes.callsign))
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) { AgentBadge(text: context.attributes.badge) }
                DynamicIslandExpandedRegion(.center) {
                    Text("\(context.attributes.callsign) needs you").font(.system(size: 15, weight: .semibold)).lineLimit(1)
                }
                DynamicIslandExpandedRegion(.trailing) { Image(systemName: "mic.fill").foregroundStyle(ask) }
                DynamicIslandExpandedRegion(.bottom) {
                    Text(context.state.question).font(.system(size: 13)).foregroundStyle(ink.opacity(0.8)).lineLimit(2)
                }
            } compactLeading: {
                AgentBadge(text: context.attributes.badge, size: 22)
            } compactTrailing: {
                Image(systemName: "mic.fill").foregroundStyle(ask)
            } minimal: {
                Image(systemName: "mic.fill").foregroundStyle(ask)
            }
            .widgetURL(OpenWith.answer(context.attributes.callsign))
        }
    }
}
