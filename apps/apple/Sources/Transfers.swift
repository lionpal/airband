// Moving a model's bytes with the system's background transfers, so a download goes on while the app is in the
// background or closed. The core decides what to fetch, where each piece goes and whether it is right; this
// only moves bytes and says how it went.
import Foundation

final class ModelTransfers: NSObject, Transfers, URLSessionDownloadDelegate, @unchecked Sendable {
    static let shared = ModelTransfers()
    static let identifier = "dev.airband.app.models"

    /// Where what happens is told: the desk, once it is made.
    weak var model: RadioModel?
    /// What the system hands over when it wakes the app for this session's events: called once they are all in.
    var wokenFor: (() -> Void)?
    private lazy var session: URLSession = {
        let config = URLSessionConfiguration.background(withIdentifier: Self.identifier)
        config.isDiscretionary = false
        config.sessionSendsLaunchEvents = true
        config.httpMaximumConnectionsPerHost = 4
        return URLSession(configuration: config, delegate: self, delegateQueue: nil)
    }()

    /// What a transfer carries with it, so it can be finished after the app was closed: its id and where its
    /// file goes.
    private struct Tag: Codable {
        let id: String
        let to: String
    }

    /// Make the session, so transfers started before the app was closed report here again.
    func open() { _ = session }

    func start(id: String, url: String, from: UInt64, until: UInt64, to: String) {
        guard let address = URL(string: url) else {
            model?.desk.transferEnded(id: id, status: nil)
            return
        }
        session.getAllTasks { tasks in
            if tasks.contains(where: { Self.tag(of: $0)?.id == id && $0.state == .running }) { return }
            var request = URLRequest(url: address)
            request.setValue("bytes=\(from)-\(until - 1)", forHTTPHeaderField: "Range")
            let task = self.session.downloadTask(with: request)
            task.taskDescription = (try? JSONEncoder().encode(Tag(id: id, to: to))).flatMap { String(data: $0, encoding: .utf8) }
            task.resume()
        }
    }

    private static func tag(of task: URLSessionTask) -> Tag? {
        task.taskDescription?.data(using: .utf8).flatMap { try? JSONDecoder().decode(Tag.self, from: $0) }
    }

    func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didWriteData bytesWritten: Int64, totalBytesWritten: Int64, totalBytesExpectedToWrite: Int64) {
        guard let tag = Self.tag(of: downloadTask) else { return }
        model?.desk.transferMoved(id: tag.id, bytes: UInt64(max(totalBytesWritten, 0)))
    }

    /// The file is the system's only until this returns: it is put where the core said, whole.
    func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didFinishDownloadingTo location: URL) {
        guard let tag = Self.tag(of: downloadTask) else { return }
        let to = URL(fileURLWithPath: tag.to)
        try? FileManager.default.removeItem(at: to)
        try? FileManager.default.moveItem(at: location, to: to)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        guard let tag = Self.tag(of: task) else { return }
        let status = error == nil ? (task.response as? HTTPURLResponse).map { UInt16(clamping: $0.statusCode) } : nil
        model?.desk.transferEnded(id: tag.id, status: status)
    }

    func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
        DispatchQueue.main.async {
            self.wokenFor?()
            self.wokenFor = nil
        }
    }
}
