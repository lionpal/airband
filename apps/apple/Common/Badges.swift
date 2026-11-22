import SwiftUI
import UIKit

/// The colours every part of airband draws with: the app, its widgets and its notifications. One place, so
/// the amber on the lock screen is the amber in the chat.
enum Colours {
    static let ground = Color(red: 0.043, green: 0.043, blue: 0.047)
    static let ink = Color(red: 0.957, green: 0.957, blue: 0.941)
    /// Amber: someone is waiting on him.
    static let ask = Color(red: 0.96, green: 0.77, blue: 0.32)
}

/// Who an agent is, at a glance: its two characters on a colour, in a square with rounded corners.
struct SquareBadge: View {
    let text: String
    let fill: Color
    var size: CGFloat = 22

    var body: some View {
        Text(text).font(.system(size: size * 0.5, weight: .bold, design: .rounded)).foregroundStyle(Colours.ground)
            .minimumScaleFactor(0.7).lineLimit(1).padding(.horizontal, 2)
            .frame(width: size, height: size).background(fill, in: RoundedRectangle(cornerRadius: size * 0.3))
            .accessibilityHidden(true)
    }
}

/// The same badge as a picture, for where the system wants one: a car's list, a notification. `corner` is
/// how far its corners are rounded and `letters` how tall its characters are, each as a share of its side;
/// where the system crops the picture to a circle itself, the corners are square and the characters smaller.
func badgePicture(_ text: String, fill: UIColor, side: CGFloat, corner: CGFloat = 0.3, letters: CGFloat = 0.5, scale: CGFloat? = nil) -> UIImage {
    let format = UIGraphicsImageRendererFormat()
    if let scale { format.scale = scale }
    return UIGraphicsImageRenderer(size: CGSize(width: side, height: side), format: format).image { _ in
        fill.setFill()
        UIBezierPath(roundedRect: CGRect(x: 0, y: 0, width: side, height: side), cornerRadius: side * corner).fill()
        let base = UIFont.systemFont(ofSize: side * letters, weight: .bold)
        let font = base.fontDescriptor.withDesign(.rounded).map { UIFont(descriptor: $0, size: base.pointSize) } ?? base
        let words = NSAttributedString(string: text, attributes: [.font: font, .foregroundColor: UIColor(Colours.ground)])
        let size = words.size()
        words.draw(at: CGPoint(x: (side - size.width) / 2, y: (side - size.height) / 2))
    }
}
