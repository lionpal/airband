import AVFoundation
import Photos
import PhotosUI
import SwiftUI
import UIKit

/// A photo or video he has attached and not yet sent: its small picture for the tray, and the id it travels
/// under once the phone has made it ready to send.
struct TrayItem: Identifiable, Equatable {
    let id = UUID()
    var thumb: UIImage?
    var video = false
    /// The id the desk gave it. Nothing while the phone is still shrinking the photo or the video.
    var attachment: String?
    var failed = false
    /// For a video: the one he picked, kept until it is sent or taken off, so it can be trimmed again.
    var source: URL?
    /// For a video: the part of it he kept, in seconds. Nothing means all of it.
    var kept: ClosedRange<Double>?
    let number: Int
}

/// A video picked from the library, copied to somewhere the app can read it.
private struct PickedMovie: Transferable {
    let url: URL
    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(contentType: .movie) { SentTransferredFile($0.url) } importing: { received in
            let copy = FileManager.default.temporaryDirectory.appendingPathComponent("picked-\(UUID().uuidString).\(received.file.pathExtension.isEmpty ? "mov" : received.file.pathExtension)")
            try FileManager.default.copyItem(at: received.file, to: copy)
            return PickedMovie(url: copy)
        }
    }
}

/// Making what he picked ready to send. A photo is sent at a size an agent can use (2400 points at most on its
/// long side), a video at 720p, so neither takes long on a phone connection. Returns the file, what to call it,
/// and a small picture of it.
enum Prepare {
    static func photo(_ data: Data, number: Int) -> (URL, String, UIImage)? {
        guard let image = UIImage(data: data) else { return nil }
        let long = max(image.size.width, image.size.height) * image.scale
        let scale = min(1, 2400 / max(long, 1))
        let size = CGSize(width: image.size.width * image.scale * scale, height: image.size.height * image.scale * scale)
        let format = UIGraphicsImageRendererFormat.default()
        format.scale = 1
        let small = UIGraphicsImageRenderer(size: size, format: format).image { _ in image.draw(in: CGRect(origin: .zero, size: size)) }
        guard let jpeg = small.jpegData(compressionQuality: 0.85) else { return nil }
        let name = "photo-\(number).jpg"
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString)-\(name)")
        guard (try? jpeg.write(to: file)) != nil else { return nil }
        return (file, name, thumb(small))
    }

    /// `kept`: the part of the video to send, in seconds; nothing sends all of it. The picked video is left as it
    /// is, so he can trim it again.
    static func video(_ source: URL, number: Int, kept: ClosedRange<Double>? = nil) async -> (URL, String, UIImage?)? {
        let asset = AVURLAsset(url: source)
        let name = "video-\(number).mp4"
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString)-\(name)")
        var picture: UIImage?
        let frames = AVAssetImageGenerator(asset: asset)
        frames.appliesPreferredTrackTransform = true
        frames.maximumSize = CGSize(width: 400, height: 400)
        let first = CMTime(seconds: kept?.lowerBound ?? 0, preferredTimescale: 600)
        if let frame = try? frames.copyCGImage(at: first, actualTime: nil) { picture = thumb(UIImage(cgImage: frame)) }
        // smaller first: a minute at 720p is around a tenth of what the camera recorded
        let whole = "video-\(number).\(source.pathExtension.isEmpty ? "mov" : source.pathExtension)"
        guard let export = AVAssetExportSession(asset: asset, presetName: AVAssetExportPreset960x540) else { return (source, whole, picture) }
        export.outputURL = file
        export.outputFileType = .mp4
        export.shouldOptimizeForNetworkUse = true
        if let kept { export.timeRange = CMTimeRange(start: first, end: CMTime(seconds: kept.upperBound, preferredTimescale: 600)) }
        await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in export.exportAsynchronously { done.resume() } }
        guard export.status == .completed else { return (source, whole, picture) }
        return (file, name, picture)
    }

    private static func thumb(_ image: UIImage) -> UIImage {
        let side: CGFloat = 200
        let scale = side / min(image.size.width, image.size.height)
        let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        let format = UIGraphicsImageRendererFormat.default()
        format.scale = 1
        return UIGraphicsImageRenderer(size: CGSize(width: side, height: side), format: format).image { _ in
            image.draw(in: CGRect(x: (side - size.width) / 2, y: (side - size.height) / 2, width: size.width, height: size.height))
        }
    }
}

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
                            // a video opens to be watched and trimmed
                            .onTapGesture { if item.source != nil { trimming = item } }
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

/// His four newest photos and videos, across the top of the + panel. A tap attaches one and marks it; a tap
/// on a marked one takes it off again. The panel stays open, so he can take several. The whole library is the
/// button under this. Shown once he has let the app see his photos; asked the first time the panel opens.
struct RecentStrip: View {
    @EnvironmentObject var model: RadioModel
    @State private var recent: [PHAsset] = []
    /// Which of them are in the tray, and as which item there.
    @State private var taken: [String: UUID] = [:]

    var body: some View {
        Group {
            if !recent.isEmpty {
                HStack(spacing: 8) {
                    ForEach(recent, id: \.localIdentifier) { asset in
                        let item = taken[asset.localIdentifier].flatMap { id in model.tray.first { $0.id == id } }
                        Button {
                            if let item {
                                model.detach(item)
                                taken[asset.localIdentifier] = nil
                            } else {
                                taken[asset.localIdentifier] = model.attach(asset)
                            }
                        } label: {
                            RecentThumb(asset: asset, chosen: item != nil)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(asset.mediaType == .video ? "Recent video" : "Recent photo")
                        .accessibilityValue(item != nil ? "Attached" : "")
                        .accessibilityHint(item != nil ? "Takes it off again" : "Attaches it")
                    }
                    // fewer than four: the ones there keep their size
                    ForEach(0..<max(0, 4 - recent.count), id: \.self) { _ in Color.clear.aspectRatio(1, contentMode: .fit) }
                }
            }
        }
        .task {
            let status = await PHPhotoLibrary.requestAuthorization(for: .readWrite)
            guard status == .authorized || status == .limited else { return }
            let newest = PHFetchOptions()
            newest.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
            newest.predicate = NSPredicate(format: "mediaType == %d OR mediaType == %d", PHAssetMediaType.image.rawValue, PHAssetMediaType.video.rawValue)
            newest.fetchLimit = 4
            var found: [PHAsset] = []
            PHAsset.fetchAssets(with: newest).enumerateObjects { asset, _, _ in found.append(asset) }
            recent = found
        }
    }
}

private struct RecentThumb: View {
    let asset: PHAsset
    let chosen: Bool
    @State private var image: UIImage?

    var body: some View {
        // a square a quarter of the row wide, whatever the picture's own shape
        Color.clear.aspectRatio(1, contentMode: .fit)
            .overlay { if let image { Image(uiImage: image).resizable().scaledToFill() } else { Palette.raised } }
            .overlay(alignment: .bottomLeading) {
                if asset.mediaType == .video {
                    Image(systemName: "video.fill").font(.system(size: 11, weight: .bold)).foregroundStyle(.white)
                        .padding(5).background(.black.opacity(0.45), in: Capsule()).padding(5)
                }
            }
            .overlay(alignment: .topTrailing) {
                if chosen {
                    Image(systemName: "checkmark.circle.fill").font(.system(size: 22, weight: .bold))
                        .symbolRenderingMode(.palette).foregroundStyle(Palette.ground, Palette.ink).padding(5)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 14))
            .overlay { if chosen { RoundedRectangle(cornerRadius: 14).strokeBorder(Palette.ink, lineWidth: 2.5) } }
        .task {
            let wanted = PHImageRequestOptions()
            wanted.isNetworkAccessAllowed = true
            wanted.deliveryMode = .opportunistic
            // called with a rough picture first and the sharp one after
            PHImageManager.default().requestImage(for: asset, targetSize: CGSize(width: 264, height: 264), contentMode: .aspectFill, options: wanted) { got, _ in
                if let got { DispatchQueue.main.async { image = got } }
            }
        }
    }
}

/// The tile in the + panel that opens his photos.
struct AttachTile: View {
    let done: () -> Void

    var body: some View {
        AttachPicker(done: done) {
            HStack(spacing: 10) {
                Image(systemName: "photo.on.rectangle.angled").font(.system(size: 18, weight: .semibold))
                Text("Photo or video").font(.system(size: 16, weight: .semibold))
                Spacer()
                Image(systemName: "plus").font(.system(size: 14, weight: .bold)).foregroundStyle(Palette.quiet)
            }
            .padding(.horizontal, 16).frame(maxWidth: .infinity, minHeight: 54)
            .background(Palette.raised, in: RoundedRectangle(cornerRadius: 16))
            .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(Palette.read.opacity(0.55), lineWidth: 1))
        }
    }
}

/// Opens the photo picker from whatever it is given to look like, and adds what he picks to what he is
/// about to say. `done` runs once something has been added.
struct AttachPicker<Label: View>: View {
    @EnvironmentObject var model: RadioModel
    @State private var picked: [PhotosPickerItem] = []
    var done: () -> Void = {}
    @ViewBuilder var label: () -> Label

    var body: some View {
        PhotosPicker(selection: $picked, maxSelectionCount: 6, matching: .any(of: [.images, .videos]), label: label)
            .buttonStyle(.plain)
            .onChange(of: picked) { _, items in
                guard !items.isEmpty else { return }
                model.attach(items)
                picked = []
                done()
            }
    }
}

extension RadioModel {
    /// He trimmed a video in the tray: what was on its way is dropped and the part he kept is sent in its place.
    @MainActor func trim(_ item: TrayItem, to range: ClosedRange<Double>?) {
        guard let i = tray.firstIndex(where: { $0.id == item.id }), let source = tray[i].source, tray[i].kept != range else { return }
        if let id = tray[i].attachment { dropUpload(id) }
        tray[i].attachment = nil
        tray[i].failed = false
        tray[i].kept = range
        let (ident, number) = (item.id, item.number)
        Task {
            let made = await Prepare.video(source, number: number, kept: range)
            await MainActor.run { self.prepared(ident, made, source: source) }
        }
    }

    /// Everything attached has reached his computer.
    var trayReady: Bool { !tray.isEmpty && tray.allSatisfy { item in view.uploads.first { $0.id == item.attachment }?.state == .ready } }

    /// He tapped one of his newest photos or videos: it goes the way a picked one does. Returns which item in
    /// the tray it became.
    @discardableResult func attach(_ asset: PHAsset) -> UUID {
        let video = asset.mediaType == .video
        let number = tray.count + 1
        let item = TrayItem(video: video, number: number)
        tray.append(item)
        if video {
            let wanted = PHVideoRequestOptions()
            wanted.isNetworkAccessAllowed = true
            PHImageManager.default().requestAVAsset(forVideo: asset, options: wanted) { movie, _, _ in
                // A copy of the app's own, as a picked video is: the tray trims its source and deletes it when the
                // item is taken off, and neither may ever touch the one in his library.
                let copy = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).mov")
                guard let url = (movie as? AVURLAsset)?.url, (try? FileManager.default.copyItem(at: url, to: copy)) != nil else {
                    DispatchQueue.main.async { self.prepared(item.id, nil, source: nil) }
                    return
                }
                Task {
                    let made = await Prepare.video(copy, number: number)
                    await MainActor.run { self.prepared(item.id, made, source: copy) }
                }
            }
        } else {
            let wanted = PHImageRequestOptions()
            wanted.isNetworkAccessAllowed = true
            wanted.deliveryMode = .highQualityFormat
            PHImageManager.default().requestImageDataAndOrientation(for: asset, options: wanted) { data, _, _, _ in
                let photo = data.flatMap { Prepare.photo($0, number: number) }
                DispatchQueue.main.async { self.prepared(item.id, photo.map { ($0.0, $0.1, $0.2) }, source: nil) }
            }
        }
        return item.id
    }

    /// He picked these from his photos: each appears in the tray at once, is made ready on the phone, then sent.
    func attach(_ picked: [PhotosPickerItem]) {
        for pick in picked {
            let video = pick.supportedContentTypes.contains { $0.conforms(to: .movie) }
            let number = tray.count + 1
            let item = TrayItem(video: video, number: number)
            tray.append(item)
            Task {
                if video, let movie = try? await pick.loadTransferable(type: PickedMovie.self) {
                    let made = await Prepare.video(movie.url, number: number)
                    await MainActor.run { self.prepared(item.id, made, source: movie.url) }
                } else if let data = try? await pick.loadTransferable(type: Data.self), let photo = Prepare.photo(data, number: number) {
                    await MainActor.run { self.prepared(item.id, (photo.0, photo.1, photo.2), source: nil) }
                } else {
                    await MainActor.run { self.prepared(item.id, nil, source: nil) }
                }
            }
        }
    }
}

/// The lightning button's panel, now behind a +: his photos first, then the things he says all the time.
/// A round button that opens the photo picker and nothing else: for adding to what he is saying while the
/// microphone is open.
struct AttachButton: View {
    var body: some View {
        AttachPicker {
            Image(systemName: "photo.on.rectangle.angled").font(.system(size: 19, weight: .semibold)).foregroundStyle(Palette.ink)
                .frame(width: 56, height: 56).frosted(Circle())
        }
        .accessibilityLabel("Attach a photo or a video")
    }
}

struct PlusButtonLabel: View {
    let open: Bool
    var body: some View {
        Image(systemName: "plus").font(.system(size: 24, weight: .semibold))
            .foregroundStyle(open ? Palette.ground : Palette.ink).rotationEffect(.degrees(open ? 45 : 0))
            .frame(width: 56, height: 56).background { if open { Circle().fill(Palette.ink) } }.frosted(Circle(), shown: !open)
    }
}

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
                        start = min(max(0, length * drag.location.x / width), end - 1)
                        seek(start)
                    })
                    .accessibilityLabel("Start").accessibilityValue(clock(start))
                handle.offset(x: x(end) - 18)
                    .gesture(DragGesture(minimumDistance: 0, coordinateSpace: .named("strip")).onChanged { drag in
                        pause()
                        end = max(min(length, length * drag.location.x / width), start + 1)
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
