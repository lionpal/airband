// Trimming a video he attached, before it is sent.
import AVFoundation
import Photos
import PhotosUI
import SwiftUI
import UIKit

/// A video layer and nothing else: no controls of the system's.
private struct PlayerSurface: UIViewRepresentable {
    let player: AVPlayer
    final class Layered: UIView {
        override class var layerClass: AnyClass { AVPlayerLayer.self }
    }
    func makeUIView(context: Context) -> Layered {
        let view = Layered()
        (view.layer as? AVPlayerLayer)?.player = player
        (view.layer as? AVPlayerLayer)?.videoGravity = .resizeAspect
        return view
    }
    func updateUIView(_ view: Layered, context: Context) {}
}

/// Watch a video he attached and keep only part of it: a strip of its frames with a handle at each end, and a
/// play button that starts at the first handle and stops at the second. Only the kept part is sent.
struct TrimView: View {
    @Environment(\.dismiss) private var dismiss
    let source: URL
    let done: (ClosedRange<Double>?) -> Void
    @State private var player: AVPlayer
    @State private var length: Double = 0
    @State private var start: Double
    @State private var end: Double
    @State private var at: Double = 0
    @State private var playing = false
    @State private var frames: [UIImage] = []
    @State private var watcher: Any?

    init(source: URL, kept: ClosedRange<Double>?, done: @escaping (ClosedRange<Double>?) -> Void) {
        self.source = source
        self.done = done
        _player = State(initialValue: AVPlayer(url: source))
        _start = State(initialValue: kept?.lowerBound ?? 0)
        _end = State(initialValue: kept?.upperBound ?? 0)
    }

    private func clock(_ seconds: Double) -> String { String(format: "%d:%02d", Int(seconds) / 60, Int(seconds) % 60) }
    private func seek(_ seconds: Double) { player.seek(to: CMTime(seconds: seconds, preferredTimescale: 600), toleranceBefore: .zero, toleranceAfter: .zero) }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Button { dismiss() } label: {
                    RoundIcon("xmark", side: 40, glyph: 15, weight: .bold)
                }
                .buttonStyle(.plain).accessibilityLabel("Cancel")
                Spacer()
                Text(length > 0 ? clock(end - start) : "").font(.system(size: 16, weight: .semibold, design: .monospaced)).foregroundStyle(Palette.soft)
                Spacer()
                Button {
                    // all of it kept is the same as never trimmed
                    done(start <= 0.05 && end >= length - 0.05 ? nil : start...end)
                    dismiss()
                } label: {
                    Text("Use").font(.system(size: 16, weight: .semibold)).foregroundStyle(Palette.ground)
                        .padding(.horizontal, 18).frame(height: 40).background(Palette.ink, in: Capsule())
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 16).padding(.top, 10)

            PlayerSurface(player: player).frame(maxWidth: .infinity, maxHeight: .infinity).padding(.vertical, 12)
                .contentShape(Rectangle()).onTapGesture { toggle() }

            Button { toggle() } label: {
                Image(systemName: playing ? "pause.fill" : "play.fill").font(.system(size: 22, weight: .bold)).foregroundStyle(Palette.ground)
                    .frame(width: 60, height: 60).background(Palette.ink, in: Circle())
            }
            .buttonStyle(.plain).accessibilityLabel(playing ? "Pause" : "Play from the start of what is kept").padding(.bottom, 16)

            strip.frame(height: 56).padding(.horizontal, 28).padding(.bottom, 26)
        }
        .background(Color.black.ignoresSafeArea())
        .foregroundStyle(Palette.ink)
        .preferredColorScheme(.dark)
        .task { await load() }
        .onDisappear {
            player.pause()
            if let watcher { player.removeTimeObserver(watcher) }
        }
    }

    /// The frames, the part outside the handles dimmed, a handle at each end to drag, and a line where it is playing.
    private var strip: some View {
        GeometryReader { box in
            let width = box.size.width
            let x = { (seconds: Double) in length > 0 ? width * seconds / length : 0 }
            ZStack(alignment: .leading) {
                HStack(spacing: 0) {
                    ForEach(Array(frames.enumerated()), id: \.offset) { _, frame in
                        Image(uiImage: frame).resizable().scaledToFill().frame(width: width / CGFloat(max(frames.count, 1)), height: 56).clipped()
                    }
                }
                .frame(width: width, alignment: .leading).background(Palette.raised).clipShape(RoundedRectangle(cornerRadius: 10))
                Color.black.opacity(0.6).frame(width: max(0, x(start))).clipShape(RoundedRectangle(cornerRadius: 10))
                Color.black.opacity(0.6).frame(width: max(0, width - x(end))).offset(x: x(end)).clipShape(RoundedRectangle(cornerRadius: 10))
                RoundedRectangle(cornerRadius: 10).strokeBorder(Palette.ask, lineWidth: 3).frame(width: max(24, x(end) - x(start))).offset(x: x(start))
                if playing || at > start { Rectangle().fill(.white).frame(width: 2, height: 64).offset(x: min(max(x(at), x(start)), x(end)) - 1) }
                handle.offset(x: x(start) - 18)
                    .gesture(DragGesture(minimumDistance: 0, coordinateSpace: .named("strip")).onChanged { drag in
                        pause()
                        start = TrimBounds.start(length * drag.location.x / width, end: end, length: length)
                        seek(start)
                    })
                    .accessibilityLabel("Start").accessibilityValue(clock(start))
                handle.offset(x: x(end) - 18)
                    .gesture(DragGesture(minimumDistance: 0, coordinateSpace: .named("strip")).onChanged { drag in
                        pause()
                        end = TrimBounds.end(length * drag.location.x / width, start: start, length: length)
                        seek(end)
                    })
                    .accessibilityLabel("End").accessibilityValue(clock(end))
            }
            .coordinateSpace(name: "strip")
        }
    }

    private var handle: some View {
        RoundedRectangle(cornerRadius: 4).fill(Palette.ask).frame(width: 10, height: 64)
            .overlay(Capsule().fill(Palette.ground.opacity(0.7)).frame(width: 2, height: 22))
            .frame(width: 36, height: 72).contentShape(Rectangle())
    }

    private func pause() {
        player.pause()
        playing = false
    }

    private func toggle() {
        if playing { return pause() }
        // from the first handle, every time
        seek(start)
        player.play()
        playing = true
    }

    private func load() async {
        let asset = AVURLAsset(url: source)
        length = ((try? await asset.load(.duration))?.seconds).flatMap { $0.isFinite ? $0 : nil } ?? 0
        if end <= 0 || end > length { end = length }
        seek(start)
        // it stops at the second handle
        watcher = player.addPeriodicTimeObserver(forInterval: CMTime(value: 1, timescale: 20), queue: .main) { time in
            at = time.seconds
            if playing, time.seconds >= end { pause() }
        }
        // eight small frames along it, made off the main thread
        let maker = AVAssetImageGenerator(asset: asset)
        maker.appliesPreferredTrackTransform = true
        maker.maximumSize = CGSize(width: 160, height: 160)
        var made: [UIImage] = []
        for i in 0..<8 where length > 0 {
            let when = CMTime(seconds: length * (Double(i) + 0.5) / 8, preferredTimescale: 600)
            if let (frame, _) = try? await maker.image(at: when) { made.append(UIImage(cgImage: frame)) }
        }
        frames = made
    }
}
