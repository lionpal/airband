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
                Text("Failed requests and sends, with times. None of your messages are in it, and it stays on this phone.")
            }
        }
        .navigationTitle("Connection").navigationBarTitleDisplayMode(.inline)
        .onAppear { reach = model.reach() }
    }

    private func line(_ title: String, _ last: String?) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
            Text(last ?? "No failures on record").font(.footnote).foregroundStyle(.secondary)
        }
    }
}
