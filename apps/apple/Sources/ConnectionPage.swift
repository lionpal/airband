import SwiftUI

/// How each computer was last reached, and the record to copy when something is wrong. All of it is what this
/// phone saw itself and kept. Nothing here is sent anywhere.
struct ConnectionPage: View {
    @EnvironmentObject var model: RadioModel
    @State private var reach: [DeskReach] = []
    @State private var copied = false

    var body: some View {
        Form {
            ForEach(reach, id: \.id) { r in
                Section(r.computer) {
                    line("Through the relay", r.relay)
                    line("On this network", r.near)
                }
            }
            Section {
                Button(copied ? "Copied" : "Copy the connection record") {
                    UIPasteboard.general.string = model.diagnostics()
                    copied = true
                }
            } footer: {
                Text("The record lists requests that failed and each try at sending what you said, with the time, the way it went and how it ended. Nothing you or your agents said is in it. It stays on this phone unless you paste it somewhere.")
            }
        }
        .navigationTitle("Connection").navigationBarTitleDisplayMode(.inline)
        .onAppear { reach = model.reach() }
    }

    private func line(_ title: String, _ last: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
            Text(last.isEmpty ? "Nothing has failed" : last).font(.footnote).foregroundStyle(.secondary)
        }
    }
}
