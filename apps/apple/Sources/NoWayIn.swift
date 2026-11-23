// Said where he types, before he does: the agent he is looking at cannot be sent messages, and the way to make
// it one that can. What he sends it anyway is kept in the chat as not delivered.
import SwiftUI

struct NoWayIn: View {
    @EnvironmentObject var model: RadioModel
    let shut: DeskNoWayIn
    @State private var taking: DeskMember?

    var body: some View {
        HStack(spacing: 12) {
            Text(shut.line).font(.footnote).foregroundStyle(Palette.soft).frame(maxWidth: .infinity, alignment: .leading)
            if shut.takeOver, let agent = model.member(shut.agent)?.agent {
                Button("Take over") { taking = agent }.font(.footnote.weight(.semibold)).foregroundStyle(Palette.ink).buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 12).padding(.vertical, 7).frosted(RoundedRectangle(cornerRadius: 12)).padding(.horizontal, 8)
        .takeOverAlert($taking)
    }
}
