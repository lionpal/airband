// Small rules of the phone's own that are not the core's: what counts as nothing typed, a time the core gives
// in milliseconds, where the handles of a trim may go, and taking a pairing link from the clipboard. Each is
// written once, here, and tested (Tests/).
import Foundation
import UIKit

extension String {
    /// The words without the spaces and line breaks around them: the one way the phone trims what he typed.
    var trimmed: String { trimmingCharacters(in: .whitespacesAndNewlines) }
    /// Nothing but spaces and line breaks: nothing typed.
    var isBlank: Bool { trimmed.isEmpty }
    /// Itself, or nothing when it is empty: for the core's calls that take nothing for "none".
    var orNone: String? { isEmpty ? nil : self }
}

extension Date {
    /// A moment the core gives in milliseconds since 1970.
    init<T: BinaryInteger>(ms: T) {
        self.init(timeIntervalSince1970: Double(ms) / 1000)
    }
}

/// Where the two handles of a trim may go: inside the clip, the start before the end, with at least a second
/// between them, or the whole clip when it is shorter than that.
enum TrimBounds {
    /// The least that can be kept: a second, or all of a clip that is shorter.
    static func least(_ length: Double) -> Double { min(1, max(0, length)) }
    /// The start, dragged to `wanted`, with the end where it is.
    static func start(_ wanted: Double, end: Double, length: Double) -> Double {
        min(max(0, wanted), max(0, end - least(length)))
    }
    /// The end, dragged to `wanted`, with the start where it is.
    static func end(_ wanted: Double, start: Double, length: Double) -> Double {
        max(min(length, wanted), min(length, start + least(length)))
    }
}

extension RadioModel {
    /// He pasted a pairing link from the clipboard: the one way, wherever the button is. What was wrong, in words
    /// for him, or nothing when it was taken.
    @discardableResult
    func pairPasted(from board: UIPasteboard = .general) -> String? {
        pair(with: board.string ?? "")
    }
}
