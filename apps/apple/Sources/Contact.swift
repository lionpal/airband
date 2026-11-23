// Writing to the person who makes airband, from the end of every list he picks from and from Settings. Who
// and what the message starts with are the core's; this opens a new text message with them.
import MessageUI
import SwiftUI

/// A row that opens a new text message to the person who makes airband.
struct ContactRow: View {
    @EnvironmentObject var model: RadioModel
    let from: ContactFrom
    /// Drawn as a row of a Settings list, not as a link under a list of cards.
    var plain = false
    @State private var writing: ContactNote?

    var body: some View {
        if let note = model.desk.contact(from: from) {
            Button(note.label) { open(note) }
                .modifier(Look(plain: plain))
                .sheet(isPresented: Binding(get: { writing != nil }, set: { if !$0 { writing = nil } })) {
                    if let writing { MessageSheet(note: writing).ignoresSafeArea() }
                }
        }
    }

    /// The message sheet where the phone can send texts; else the Messages app, with the same message.
    private func open(_ note: ContactNote) {
        if MFMessageComposeViewController.canSendText() {
            writing = note
            return
        }
        var parts = URLComponents()
        parts.scheme = "sms"
        parts.path = note.to
        if !note.body.isEmpty { parts.queryItems = [URLQueryItem(name: "body", value: note.body)] }
        if let url = parts.url { UIApplication.shared.open(url) }
    }

    private struct Look: ViewModifier {
        let plain: Bool
        func body(content: Content) -> some View {
            if plain {
                content
            } else {
                content.font(.system(size: 15, weight: .semibold)).foregroundStyle(Palette.quiet).buttonStyle(.plain)
                    .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading).padding(.horizontal, 2)
            }
        }
    }
}

/// The system's sheet for a new text message, filled in.
private struct MessageSheet: UIViewControllerRepresentable {
    let note: ContactNote
    @Environment(\.dismiss) private var dismiss

    func makeCoordinator() -> Done { Done { dismiss() } }
    func makeUIViewController(context: Context) -> MFMessageComposeViewController {
        let sheet = MFMessageComposeViewController()
        sheet.recipients = [note.to]
        sheet.body = note.body
        sheet.messageComposeDelegate = context.coordinator
        return sheet
    }
    func updateUIViewController(_ controller: MFMessageComposeViewController, context: Context) {}

    final class Done: NSObject, MFMessageComposeViewControllerDelegate {
        let close: () -> Void
        init(_ close: @escaping () -> Void) { self.close = close }
        func messageComposeViewController(_ controller: MFMessageComposeViewController, didFinishWith result: MessageComposeResult) { close() }
    }
}
