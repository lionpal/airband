import SwiftUI

/// Shows what the working teams are doing, one at a time: picks a team, types its agent's latest step out
/// quickly, holds it for a moment, takes it away, and moves on. Nothing here is kept or read aloud.
@MainActor final class Ticker: ObservableObject {
    @Published var room: String?
    @Published var shown = ""
    private var turn = 0
    /// The last step shown for each team.
    private var told: [String: String] = [:]

    func run(_ rooms: @escaping () -> [DeskRoom]) async {
        while !Task.isCancelled {
            // only what is new: a step a team is still on is shown once, not typed out again and again
            let busy = rooms().filter { $0.doing != nil && told[$0.name] != $0.doing }
            guard !busy.isEmpty, !UIAccessibility.isReduceMotionEnabled else {
                try? await Task.sleep(for: .milliseconds(700))
                continue
            }
            let pick = busy[turn % busy.count]
            turn += 1
            told[pick.name] = pick.doing
            shown = ""
            room = pick.name
            for letter in pick.doing ?? "" {
                shown.append(letter)
                try? await Task.sleep(for: .milliseconds(18))
            }
            try? await Task.sleep(for: .milliseconds(1700))
            room = nil
            try? await Task.sleep(for: .milliseconds(900))
        }
    }
}
/// The bubble itself: small, with a tail up to the chip it comes out of and a caret after the last letter.
struct DoingBubble: View {
    let text: String
    /// It hangs from the chip's right end and grows leftwards, for a chip near the right edge of the screen.
    var right = false
    /// Where its tail is: up to the chip it comes out of, or off the side of the screen its chip has scrolled to.
    var tail: Edge = .top
    @State private var caret = true

    var body: some View {
        // as wide as its words, up to most of a phone's width, whatever the width of the chip above it
        HStack(spacing: 0) {
            if right { Spacer(minLength: 0) }
            bubble
            if !right { Spacer(minLength: 0) }
        }
        .frame(width: min(300, UIScreen.main.bounds.width - 44))
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    private var bubble: some View {
        HStack(spacing: 2) {
            Text(text).font(.system(size: 12, weight: .medium)).foregroundStyle(Palette.soft).lineLimit(1).truncationMode(.tail)
            Rectangle().fill(Palette.signal).frame(width: 1.5, height: 12).opacity(caret ? 1 : 0)
                .onAppear { withAnimation(.linear(duration: 0.01).delay(0.35).repeatForever(autoreverses: true)) { caret = false } }
        }
        .padding(.horizontal, 9).padding(.vertical, 5)
        .background(Color(red: 0.118, green: 0.129, blue: 0.133), in: RoundedRectangle(cornerRadius: 9))
        .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(Color(red: 0.18, green: 0.192, blue: 0.2), lineWidth: 1))
        .overlay(alignment: tail == .top ? (right ? .topTrailing : .topLeading) : (tail == .trailing ? .trailing : .leading)) {
            Rectangle().fill(Color(red: 0.118, green: 0.129, blue: 0.133)).frame(width: 7, height: 7).rotationEffect(.degrees(45))
                .offset(x: tail == .top ? (right ? -12 : 12) : (tail == .trailing ? 3.5 : -3.5), y: tail == .top ? -3.5 : 0)
        }
    }
}
