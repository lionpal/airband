import SwiftUI

// The small marks that say who and what at a glance: an agent's badge, its tool and computer, a count of
// what is unread, the dot for what a team is doing.

/// How tall the buttons over the bottom of the chat are, told upward by the buttons themselves.
struct BarHeight: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}
/// Who an agent is, at a glance: two characters on its team's colour.
struct Badge: View {
    let text: String
    let colour: UInt8
    var size: CGFloat = 22

    var body: some View {
        SquareBadge(text: text, fill: Palette.team(colour), size: size)
    }
}
/// Two small pictures for an agent: the coding tool it is, and the computer it is on when he has several.
struct AgentMarks: View {
    @EnvironmentObject var model: RadioModel
    let agent: DeskMember

    var body: some View {
        HStack(spacing: 8) {
            if UIImage(named: "harness-\(agent.harness)") != nil {
                Image("harness-\(agent.harness)").resizable().scaledToFit().frame(width: 15, height: 15)
            } else {
                Image(systemName: "terminal").font(.system(size: 13, weight: .medium))
            }
            if model.view.stations.count > 1, !agent.station.isEmpty {
                Image(systemName: model.icon(of: agent.station)).font(.system(size: 14, weight: .medium))
            }
        }
        .foregroundStyle(Palette.quiet)
        .accessibilityElement().accessibilityLabel([agent.harness, agent.station].filter { !$0.isEmpty }.joined(separator: ", "))
    }
}
/// How many things have been said somewhere he is not looking: a small count. Nothing when there are none.
struct Unread: View {
    let count: UInt32

    var body: some View {
        if count > 0 {
            Text(count > 99 ? "99+" : "\(count)").font(.system(size: 11, weight: .bold, design: .rounded)).monospacedDigit()
                .foregroundStyle(Palette.ground).padding(.horizontal, 5).frame(minWidth: 18, minHeight: 18)
                .background(Palette.ink, in: Capsule()).accessibilityHidden(true)
        }
    }
}
/// What a team or an agent is doing, as a dot: green and pulsing while working, amber when waiting on him, gray
/// and still when it has stopped without finishing, an empty ring when it has finished and has nothing on.
struct StateDot: View {
    let state: String
    /// Someone here is at work, whatever else the dot says: a team with a question waiting and an agent busy
    /// is amber with the green ring going.
    var working = false
    @State private var out = false

    private var color: Color {
        switch state {
        case "asking": Palette.ask
        case "working": Palette.signal
        case "stalled": Palette.quiet
        default: Palette.dim
        }
    }

    var body: some View {
        Circle().fill(state == "idle" || state.isEmpty ? Color.clear : color).frame(width: 8, height: 8)
            .overlay { if state == "idle" || state.isEmpty { Circle().strokeBorder(Palette.dim, lineWidth: 1.5) } }
            .overlay {
                if state == "working" || working {
                    Circle().stroke(Palette.signal.opacity(out ? 0 : 0.6), lineWidth: 2).scaleEffect(out ? 2.4 : 1)
                        .onAppear { withAnimation(.easeOut(duration: 1.6).repeatForever(autoreverses: false)) { out = true } }
                }
            }
            .accessibilityHidden(true)
    }
}
/// The mark with a house where the plane is: the same three waves, drawn to the measure of the logo.
struct HomeMark: View {
    var body: some View {
        Canvas { ctx, size in
            let k = size.width / 512
            // the logo's waves: arcs about one centre, a quarter turn wide, opening upwards
            let centre = CGPoint(x: 256 * k, y: 350 * k)
            for radius in [132.0, 214.0, 296.0] {
                var arc = Path()
                arc.addArc(center: centre, radius: radius * 0.88 * k, startAngle: .degrees(-135), endAngle: .degrees(-45), clockwise: false)
                ctx.stroke(arc, with: .color(Palette.signal), style: StrokeStyle(lineWidth: 40 * 0.88 * k, lineCap: .round))
            }
            // the house: a roof and a body, where the plane sits
            var house = Path()
            let (cx, top, w, h) = (256 * k, 300 * k, 104 * k, 104 * k)
            house.move(to: CGPoint(x: cx, y: top))
            house.addLine(to: CGPoint(x: cx + w / 2 + 10 * k, y: top + h * 0.46))
            house.addLine(to: CGPoint(x: cx + w / 2 - 9 * k, y: top + h * 0.46))
            house.addLine(to: CGPoint(x: cx + w / 2 - 9 * k, y: top + h))
            house.addLine(to: CGPoint(x: cx - w / 2 + 9 * k, y: top + h))
            house.addLine(to: CGPoint(x: cx - w / 2 + 9 * k, y: top + h * 0.46))
            house.addLine(to: CGPoint(x: cx - w / 2 - 10 * k, y: top + h * 0.46))
            house.closeSubpath()
            ctx.fill(house, with: .color(Palette.ink))
            ctx.stroke(house, with: .color(Palette.ink), style: StrokeStyle(lineWidth: 8 * k, lineJoin: .round))
        }
        .background(Palette.ground)
    }
}
