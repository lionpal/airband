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
