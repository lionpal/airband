// The tray of what he attached, above the composer.
import AVFoundation
import SwiftUI
import UIKit

/// What he has attached, above the bar: each with its picture, a ring that fills as it reaches his computer, and
/// a cross to take it off again. When they are all there, an arrow sends them as they are.
struct Tray: View {
    @EnvironmentObject var model: RadioModel
    /// Its own send button: for sending the files with nothing said. Hidden while he is talking or typing, when
    /// the one send button is the microphone's or the text box's and takes the files along.
    var sends = true
    @State private var trimming: TrayItem?

    var body: some View {
        HStack(spacing: 10) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 10) {
                    ForEach(model.tray) { item in
                        TrayThumb(item: item, upload: model.view.uploads.first { $0.id == item.attachment })
                            .transition(.scale(scale: 0.6).combined(with: .opacity))
                            // one that did not go says why; a video opens to be watched and trimmed
                            .onTapGesture {
                                if let why = item.why ?? model.view.uploads.first(where: { $0.id == item.attachment })?.why { model.problem = why }
                                if item.source != nil { trimming = item }
                            }
                            .accessibilityAddTraits(item.source != nil ? .isButton : [])
                            .accessibilityHint(item.source != nil ? "Watch and trim" : "")
                    }
                }
                .padding(.leading, 16).padding(.vertical, 8)
            }
            .scrollClipDisabled()
            // send them with nothing said: "look at this"
            if sends {
                Button { model.send("") } label: {
                    Image(systemName: "arrow.up").font(.system(size: 19, weight: .bold)).foregroundStyle(Palette.ground)
                        .frame(width: 46, height: 46).background(model.trayReady ? Palette.ink : Palette.dim, in: Circle())
                }
                .buttonStyle(.plain).disabled(!model.trayReady).padding(.trailing, 16)
                .accessibilityLabel("Send what is attached").accessibilityHint(model.trayReady ? "" : "Still on its way to your computer")
            }
        }
        .animation(.spring(response: 0.32, dampingFraction: 0.8), value: model.tray)
        .fullScreenCover(item: $trimming) { item in
            if let source = item.source { TrimView(source: source, kept: item.kept) { range in model.trim(item, to: range) } }
        }
    }
}

private struct TrayThumb: View {
    @EnvironmentObject var model: RadioModel
    let item: TrayItem
    let upload: DeskUpload?
    private var ready: Bool { upload?.state == .ready }
    private var failed: Bool { item.failed || upload?.state == .failed }
    /// How much of it has reached his computer, from 0 to 1. Nothing while the phone is still preparing it.
    private var progress: Double? {
        guard let upload, upload.total > 0 else { return nil }
        return min(1, Double(upload.sent) / Double(upload.total))
    }

    var body: some View {
        ZStack {
            if let thumb = item.thumb {
                Image(uiImage: thumb).resizable().scaledToFill()
            } else {
                Palette.raised
            }
            // dimmed until it has arrived, with the ring over it
            if !ready {
                Color.black.opacity(0.45)
                if failed {
                    Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 20, weight: .semibold)).foregroundStyle(Palette.ask)
                } else if let progress {
                    Circle().stroke(.white.opacity(0.25), lineWidth: 3).frame(width: 30, height: 30)
                    Circle().trim(from: 0, to: max(0.04, progress)).stroke(.white, style: StrokeStyle(lineWidth: 3, lineCap: .round))
                        .frame(width: 30, height: 30).rotationEffect(.degrees(-90)).animation(.linear(duration: 0.25), value: progress)
                } else {
                    ProgressView().tint(.white)
                }
            }
            if item.video, ready {
                Image(systemName: "play.fill").font(.system(size: 16, weight: .bold)).foregroundStyle(.white).shadow(color: .black.opacity(0.5), radius: 3)
            }
        }
        .frame(width: 68, height: 68).clipShape(RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(.white.opacity(0.08), lineWidth: 1))
        .shadow(color: .black.opacity(0.35), radius: 8, y: 4)
        .overlay(alignment: .topTrailing) {
            Button { model.detach(item) } label: {
                RoundIcon("xmark", side: 22, glyph: 10, weight: .heavy).foregroundStyle(Palette.ink)
                    .overlay(Circle().strokeBorder(Palette.ground, lineWidth: 2))
                    .frame(width: 36, height: 36) // a finger's worth to hit
            }
            .buttonStyle(.plain).offset(x: 13, y: -13).accessibilityLabel("Remove")
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(item.video ? "Video" : "Photo")
        .accessibilityValue(failed ? "did not send" : ready ? "ready" : "sending")
    }
}
