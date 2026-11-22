import SwiftUI

extension View {
    /// Asks before an agent he started in a terminal is taken over, and takes it over when he says so.
    func takeOverAlert(_ agent: Binding<DeskMember?>) -> some View {
        modifier(TakeOverAlert(agent: agent))
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
