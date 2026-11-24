// What he attached, made ready to send: a photo or video picked, copied and shrunk, and handed to the core.
import AVFoundation
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
    /// Why the phone could not make it ready to send, in words for him.
    var why: String?
    var failed: Bool { why != nil }
    /// Which making-ready of it counts: each trim starts a new one, and only the newest is sent. One that ends
    /// after a newer one began is thrown away, however it ends.
    var making = UUID()
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

/// Making what he picked ready to send. A photo is sent at a size an agent can use (2400 pixels at most on its
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
    static func video(_ source: URL, number: Int, kept: ClosedRange<Double>? = nil) async -> Made {
        let asset = AVURLAsset(url: source)
        let name = "video-\(number).mp4"
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString)-\(name)")
        var picture: UIImage?
        let frames = AVAssetImageGenerator(asset: asset)
        frames.appliesPreferredTrackTransform = true
        frames.maximumSize = CGSize(width: 400, height: 400)
        let first = CMTime(seconds: kept?.lowerBound ?? 0, preferredTimescale: 600)
        if let frame = try? await frames.image(at: first).image { picture = thumb(UIImage(cgImage: frame)) }
        // smaller first: a minute at 960x540 is around a tenth of what the camera recorded
        // with no way to make it smaller it is not sent: the whole recording never goes in its place, and the tray
        // shows it failed
        guard let export = AVAssetExportSession(asset: asset, presetName: AVAssetExportPreset960x540) else { return .failure(Unmade(why: Unmade.notSmaller)) }
        export.shouldOptimizeForNetworkUse = true
        if let kept { export.timeRange = CMTimeRange(start: first, end: CMTime(seconds: kept.upperBound, preferredTimescale: 600)) }
        // one that could not be made is not sent as the whole recording: the tray shows it failed
        do { try await export.export(to: file, as: .mp4) } catch { return .failure(Unmade(why: Unmade.notSmaller)) }
        return .success((file, name, picture))
    }

    /// What came of making something ready: the file, what to call it and its small picture, or why not.
    typealias Made = Result<(URL, String, UIImage?), Unmade>
    struct Unmade: Error {
        let why: String
        static let notSmaller = "This video could not be made small enough to send."
        static let unread = "This could not be read on the phone."
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
    /// Trimmed twice quickly, the second trim wins: the first one's clip is thrown away when it is done, and
    /// only one is ever sent.
    @MainActor func trim(_ item: TrayItem, to range: ClosedRange<Double>?) {
        guard let i = tray.firstIndex(where: { $0.id == item.id }), let source = tray[i].source, tray[i].kept != range else { return }
        if let id = tray[i].attachment { dropUpload(id) }
        tray[i].attachment = nil
        tray[i].why = nil
        tray[i].kept = range
        tray[i].making = UUID()
        let (ident, number, making) = (item.id, item.number, tray[i].making)
        Task {
            let made = await Prepare.video(source, number: number, kept: range)
            await MainActor.run { self.prepared(ident, made, source: source, making: making) }
        }
    }

    /// What has gone with a message has left the desk's list: it leaves the tray too, with the video it was
    /// trimmed from.
    func letGoOfSent() {
        guard !tray.isEmpty else { return }
        let gone = tray.filter { item in item.attachment.map { id in !view.uploads.contains { $0.id == id } } ?? false }
        gone.forEach { if let source = $0.source { try? FileManager.default.removeItem(at: source) } }
        tray.removeAll { item in gone.contains { $0.id == item.id } }
    }

    /// The number the next thing he attaches is called by: one past the highest in the tray, so a removal
    /// never lets two have the same name.
    private var nextNumber: Int { (tray.map(\.number).max() ?? 0) + 1 }

    /// Everything attached has reached his computer.
    var trayReady: Bool { !tray.isEmpty && tray.allSatisfy { item in view.uploads.first { $0.id == item.attachment }?.state == .ready } }

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
        await MainActor.run { self.prepared(item.id, made, source: copy, making: item.making) }
    }
    /// A photo made ready to send, or why it was not.
    private func ready(_ item: TrayItem, photo data: Data?) async {
        let made: Prepare.Made = data.flatMap { Prepare.photo($0, number: item.number) }.map { .success(($0.0, $0.1, $0.2)) } ?? .failure(Prepare.Unmade(why: Prepare.Unmade.unread))
        await MainActor.run { self.prepared(item.id, made, source: nil, making: item.making) }
    }
}
