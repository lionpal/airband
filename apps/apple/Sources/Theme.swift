import SwiftUI

// How the app looks: its colours, the frosted back of what sits over the chat, and the small pieces of look
// and behaviour that several screens share.

/// The colours. Dark first. Flat: no gradients or shadows. Each of the three accents means one thing.
enum Palette {
    static let ground = Colours.ground
    static let ink = Colours.ink
    /// Green: someone is working; and the answer an agent itself recommends.
    static let signal = Color(red: 0.24, green: 0.86, blue: 0.59)
    /// Amber: someone is waiting on him.
    static let ask = Colours.ask
    static let card = Color(red: 0.078, green: 0.090, blue: 0.086)
    static let raised = Color(red: 0.149, green: 0.161, blue: 0.165)
    static let field = Color(red: 0.102, green: 0.106, blue: 0.110)
    static let quiet = Color(red: 0.557, green: 0.557, blue: 0.576)
    static let soft = Color(red: 0.788, green: 0.788, blue: 0.773)
    static let dim = Color(red: 0.333, green: 0.341, blue: 0.353)
    /// Light blue: something to read in full.
    static let read = Color(red: 0.36, green: 0.80, blue: 0.95)
    /// A colour for each team, in the order the tower hands them out: blue, orange, violet, magenta, and last the
    /// two nearest the colours that already mean working and waiting. Checked as a set for telling apart,
    /// including by people who do not see red and green apart; a badge's letters say who it is in any case.
    static func team(_ place: UInt8?) -> Color { Color(uiColor: TeamColours.ui(place)) }
}
/// The back of something that sits over the chat: the chat shows through it, blurred and darkened enough that
/// what is on top still reads. `strength` is how much of the dark is laid over the blur; words need more than a button.
struct Frost<S: Shape>: ViewModifier {
    let shape: S
    let strength: Double
    let shown: Bool
    func body(content: Content) -> some View {
        content.background { if shown { shape.fill(.ultraThinMaterial).overlay(shape.fill(Palette.field.opacity(strength))).environment(\.colorScheme, .dark) } }
    }
}
extension View {
    func frosted<S: Shape>(_ shape: S, strength: Double = 0.5, shown: Bool = true) -> some View { modifier(Frost(shape: shape, strength: strength, shown: shown)) }
}
/// A small round button's face: one symbol on a raised circle. How big each is stays its caller's to say.
struct RoundIcon: View {
    let symbol: String
    let side: CGFloat
    let glyph: CGFloat
    let weight: Font.Weight
    init(_ symbol: String, side: CGFloat, glyph: CGFloat, weight: Font.Weight) {
        (self.symbol, self.side, self.glyph, self.weight) = (symbol, side, glyph, weight)
    }
    var body: some View {
        Image(systemName: symbol).font(.system(size: glyph, weight: weight)).frame(width: side, height: side).background(Palette.raised, in: Circle())
    }
}
extension View {
    /// A page of the app's own: the dark ground, the light ink, and dark whatever the phone is set to.
    func darkPage() -> some View {
        background(Palette.ground.ignoresSafeArea()).foregroundStyle(Palette.ink).preferredColorScheme(.dark)
    }
    /// Asks before an agent he started in a terminal is taken over, and takes it over when he says so.
    func takeOverAlert(_ agent: Binding<DeskMember?>) -> some View {
        modifier(TakeOverAlert(agent: agent))
    }
}
extension Binding {
    /// Whether an optional holds something, for an alert or a sheet shown while it does. Set to false, it
    /// lets go of what it held.
    func isSet<T>() -> Binding<Bool> where Value == T? {
        Binding<Bool>(get: { wrappedValue != nil }, set: { if !$0 { wrappedValue = nil } })
    }
}
/// The question put before a take-over, in one place: what it means, and the two answers.
struct TakeOverAlert: ViewModifier {
    @EnvironmentObject var model: RadioModel
    @Binding var agent: DeskMember?
    func body(content: Content) -> some View {
        content.alert("Take over \(agent?.callsign ?? "")?", isPresented: $agent.isSet()) {
            Button("Take over") {
                if let agent { model.takeOver(agent) }
                agent = nil
            }
            Button("Cancel", role: .cancel) { agent = nil }
        } message: {
            Text("Its terminal session ends, and the same conversation carries on run by airband. What you say then reaches it as your own words.")
        }
    }
}
