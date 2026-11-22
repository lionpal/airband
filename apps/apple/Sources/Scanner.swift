import AVFoundation
import SwiftUI
import VisionKit

/// The camera, looking for the code `airband pair` shows on the computer. Hands back what the code says.
struct CodeScanner: UIViewControllerRepresentable {
    let found: (String) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(found: found) }

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let scanner = DataScannerViewController(recognizedDataTypes: [.barcode(symbologies: [.qr])], qualityLevel: .balanced, recognizesMultipleItems: false, isHighFrameRateTrackingEnabled: false, isHighlightingEnabled: true)
        scanner.delegate = context.coordinator
        try? scanner.startScanning()
        return scanner
    }

    func updateUIViewController(_ scanner: DataScannerViewController, context: Context) {}

    static func dismantleUIViewController(_ scanner: DataScannerViewController, coordinator: Coordinator) { scanner.stopScanning() }

    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let found: (String) -> Void
        private var done = false
        init(found: @escaping (String) -> Void) { self.found = found }

        func dataScanner(_ scanner: DataScannerViewController, didAdd added: [RecognizedItem], allItems: [RecognizedItem]) {
            for item in added {
                // only a pairing code: anything else in view is left alone
                if case .barcode(let code) = item, let text = code.payloadStringValue, isPairingLink(link: text), !done {
                    done = true
                    found(text)
                }
            }
        }
    }
}

/// Full screen: the camera with one line saying what to point it at. Pairs and closes when it sees the code.
struct ScanSheet: View {
    @EnvironmentObject var model: RadioModel
    @Environment(\.dismiss) private var dismiss
    @State private var allowed: Bool?

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            if allowed == true, DataScannerViewController.isSupported, DataScannerViewController.isAvailable {
                CodeScanner { link in
                    model.pair(with: link)
                    dismiss()
                }
                .ignoresSafeArea()
            } else if allowed != nil {
                Text("airband can\u{2019}t use the camera. Allow it in Settings, or paste the link.")
                    .font(.system(size: 17)).foregroundStyle(.white.opacity(0.8)).multilineTextAlignment(.center).padding(40)
            }
        }
        .overlay(alignment: .top) {
            HStack {
                (Text("Point at the code ") + Text("airband pair").font(.system(size: 15, weight: .medium, design: .monospaced)) + Text(" shows"))
                    .font(.system(size: 16, weight: .medium)).foregroundStyle(.white)
                    .padding(.horizontal, 14).padding(.vertical, 9).background(.black.opacity(0.55), in: Capsule())
                Spacer()
                Button { dismiss() } label: {
                    Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(.white)
                        .frame(width: 40, height: 40).background(.black.opacity(0.55), in: Circle())
                }
                .buttonStyle(.plain).accessibilityLabel("Close")
            }
            .padding(.horizontal, 16).padding(.top, 10)
        }
        .preferredColorScheme(.dark)
        .task { allowed = await AVCaptureDevice.requestAccess(for: .video) }
    }
}
