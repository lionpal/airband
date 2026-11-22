// What he attached, made ready to send: a photo or video picked, copied and shrunk, and handed to the core.
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
            let copy = FileManager.default.temporaryDirectory.appendingPathComponent("picked-\(UUID().uuidString)" + Prepare.ending(received.file))
            try FileManager.default.copyItem(at: received.file, to: copy)
            return PickedMovie(url: copy)
        }
    }
}

/// Making what he picked ready to send. A photo is sent at a size an agent can use (2400 points at most on its
/// long side), a video at 960x540, so neither takes long on a phone connection. Returns the file, what to call it,
/// and a small picture of it.
enum Prepare {
    /// The ending a file's name needs to say what it is, with its dot: its own, or the one its type has as the
    /// system reads it. Nothing when neither says.
    static func ending(_ file: URL) -> String {
        let own = file.pathExtension.isEmpty ? (try? file.resourceValues(forKeys: [.contentTypeKey]))?.contentType?.preferredFilenameExtension : file.pathExtension
        return own.map { "." + $0 } ?? ""
    }
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
        // smaller first: a minute at 960x540 is around a tenth of what the camera recorded
        let whole = "video-\(number)" + ending(source)
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

    /// The number the next thing he attaches is called by: one past the highest in the tray, so a removal
    /// never lets two have the same name.
    private var nextNumber: Int { (tray.map(\.number).max() ?? 0) + 1 }

    /// Everything attached has reached his computer.
    var trayReady: Bool { !tray.isEmpty && tray.allSatisfy { item in view.uploads.first { $0.id == item.attachment }?.state == .ready } }

    /// He tapped one of his newest photos or videos: it goes the way a picked one does. Returns which item in
    /// the tray it became.
    @discardableResult func attach(_ asset: PHAsset) -> UUID {
        let item = newItem(video: asset.mediaType == .video)
        if item.video {
            let wanted = PHVideoRequestOptions()
            wanted.isNetworkAccessAllowed = true
            PHImageManager.default().requestAVAsset(forVideo: asset, options: wanted) { movie, _, _ in
                // A copy of the app's own, as a picked video is: the tray trims its source and deletes it when the
                // item is taken off, and neither may ever touch the one in his library.
                guard let url = (movie as? AVURLAsset)?.url else { return self.failed(item) }
                let copy = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + Prepare.ending(url))
                guard (try? FileManager.default.copyItem(at: url, to: copy)) != nil else { return self.failed(item) }
                Task { await self.ready(item, video: copy) }
            }
        } else {
            let wanted = PHImageRequestOptions()
            wanted.isNetworkAccessAllowed = true
            wanted.deliveryMode = .highQualityFormat
            PHImageManager.default().requestImageDataAndOrientation(for: asset, options: wanted) { data, _, _, _ in
                Task { await self.ready(item, photo: data) }
            }
        }
        return item.id
    }

    /// He picked these from his photos: each appears in the tray at once, is made ready on the phone, then sent.
    func attach(_ picked: [PhotosPickerItem]) {
        for pick in picked {
            let item = newItem(video: pick.supportedContentTypes.contains { $0.conforms(to: .movie) })
            Task {
                if item.video, let movie = try? await pick.loadTransferable(type: PickedMovie.self) {
                    await ready(item, video: movie.url)
                } else {
                    await ready(item, photo: try? await pick.loadTransferable(type: Data.self))
                }
            }
        }
    }

    /// A new item in the tray, shown at once while it is made ready.
    private func newItem(video: Bool) -> TrayItem {
        let item = TrayItem(video: video, number: nextNumber)
        tray.append(item)
        return item
    }
    /// A video of the app's own, made ready to send: the tray keeps it as the source to trim again.
    private func ready(_ item: TrayItem, video copy: URL) async {
        let made = await Prepare.video(copy, number: item.number)
        await MainActor.run { self.prepared(item.id, made, source: copy) }
    }
    /// A photo made ready to send; nothing when it could not be read.
    private func ready(_ item: TrayItem, photo data: Data?) async {
        let photo = data.flatMap { Prepare.photo($0, number: item.number) }
        await MainActor.run { self.prepared(item.id, photo.map { ($0.0, $0.1, $0.2) }, source: nil) }
    }
    /// What he attached could not be had at all.
    private func failed(_ item: TrayItem) {
        DispatchQueue.main.async { self.prepared(item.id, nil, source: nil) }
    }
}
