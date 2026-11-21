// Pictures that come with a message: a row of small ones under the text, and a full-screen view with zoom.
// The Rust core fetches each picture once and keeps it as a file; what is shown here is decoded from that file
// at the size it is drawn, so a row of screenshots costs a few hundred kilobytes of memory and not tens of megabytes.
import ImageIO
import SwiftUI
import UIKit

enum PictureLoader {
    private static let cache: NSCache<NSString, UIImage> = {
        let c = NSCache<NSString, UIImage>()
        c.totalCostLimit = 24 * 1024 * 1024
        return c
    }()

    /// The picture in this file, decoded no larger than `side` pixels on its long side.
    static func image(_ path: String, side: CGFloat) -> UIImage? {
        let key = "\(path)#\(Int(side))" as NSString
        if let hit = cache.object(forKey: key) { return hit }
        guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true, kCGImageSourceThumbnailMaxPixelSize: side,
        ]
        guard let cg = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        let image = UIImage(cgImage: cg)
        cache.setObject(image, forKey: key, cost: cg.bytesPerRow * cg.height)
        return image
    }
}

/// One picture at a given size: grey until it has been fetched and decoded.
struct PictureImage: View {
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage
    let p: ChatPicture
    /// The long side to decode at, in pixels.
    let side: CGFloat
    var fill = true
    @State private var image: UIImage?

    var body: some View {
        ZStack {
            if let image {
                Image(uiImage: image).resizable().aspectRatio(contentMode: fill ? .fill : .fit)
            } else {
                Palette.raised
                Image(systemName: "photo").font(.system(size: 20)).foregroundStyle(Palette.quiet)
            }
        }
        .onAppear { model.loadPicture(m, p) }
        .task(id: model.pictureFiles[p.id]) {
            guard let path = model.pictureFiles[p.id] else { return }
            let side = side
            image = await Task.detached(priority: .userInitiated) { PictureLoader.image(path, side: side) }.value
        }
    }
}

/// The pictures of one message in a row. Tap one to see it full screen.
struct PictureStrip: View {
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage
    var height: CGFloat = 132
    /// Laid out as wide as its pictures and no wider, without scrolling: for a few pictures that must line up
    /// with something else.
    var still = false
    @State private var viewing: Viewing?

    struct Viewing: Identifiable {
        let id: String
    }

    private func width(_ p: ChatPicture) -> CGFloat {
        let ratio = p.height > 0 ? CGFloat(p.width) / CGFloat(p.height) : 1
        // a phone screenshot is shown whole; only a very tall or very wide picture is cropped in the row
        return min(max(height * ratio, height * 0.4), height * 2.4)
    }

    private var row: some View {
        HStack(spacing: 8) {
            ForEach(still ? Array(m.pictures.prefix(3)) : m.pictures, id: \.id) { p in
                Button { viewing = Viewing(id: p.id) } label: {
                    PictureImage(m: m, p: p, side: 720)
                        .frame(width: still ? min(width(p), 96) : width(p), height: height).clipShape(RoundedRectangle(cornerRadius: 14))
                }
                .buttonStyle(.plain).accessibilityLabel("Picture. Opens full screen.")
            }
        }
    }

    var body: some View {
        Group {
            if still { row } else { ScrollView(.horizontal, showsIndicators: false) { row } }
        }
        .fullScreenCover(item: $viewing) { v in PictureViewer(m: m, start: v.id).environmentObject(model) }
    }
}

/// Full screen: swipe between a message's pictures, pinch or double-tap to zoom, drag to move around; a tap, or a
/// swipe up or down, closes it.
struct PictureViewer: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage
    @State var start: String
    /// How far he has dragged the picture away, up or down. The chat comes through behind as it goes.
    @State private var pull: CGFloat = 0
    private var listening: Bool { model.listening }
    private var away: Double { min(abs(pull) / 260, 1) }

    var body: some View {
        ZStack(alignment: .topTrailing) {
            Color.black.opacity(1 - 0.75 * away).ignoresSafeArea()
            TabView(selection: $start) {
                ForEach(m.pictures, id: \.id) { p in Zoomable(m: m, p: p, pull: $pull, close: { dismiss() }).tag(p.id) }
            }
            .tabViewStyle(.page(indexDisplayMode: m.pictures.count > 1 ? .automatic : .never))
            .ignoresSafeArea()
            Button { dismiss() } label: {
                Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(.white)
                    .frame(width: 38, height: 38).background(.white.opacity(0.18), in: Circle())
                    .frame(width: 52, height: 52)
            }
            .buttonStyle(.plain).padding(.trailing, 10).padding(.top, 4).accessibilityLabel("Close").opacity(1 - away)
        }
        // the talk button is here too: he looks at the picture and says what he thinks, and it goes to the agent
        // that sent it
        .overlay(alignment: .bottom) {
            VStack(spacing: 8) {
                Text(listening ? (model.hearing.isEmpty ? "listening" : model.hearing) : "tell \(m.from)").font(.system(size: 14, weight: .medium)).foregroundStyle(.white.opacity(0.7))
                    .lineLimit(2).truncationMode(.head).multilineTextAlignment(.center).padding(.horizontal, 24)
                Button { Task { await model.talk(about: m) } } label: {
                    Image(systemName: listening ? "arrow.up" : "mic.fill").font(.system(size: 28, weight: .semibold)).foregroundStyle(Palette.ground)
                        .frame(width: 76, height: 76).background(listening ? Palette.signal : Palette.ink, in: Circle())
                }
                .buttonStyle(.plain).accessibilityLabel(listening ? "Send what you said" : "Talk to \(m.from) about this picture")
            }
            .padding(.bottom, 26).opacity(1 - away)
        }
        .presentationBackground(.clear)
        .preferredColorScheme(.dark)
    }
}

private struct Zoomable: View {
    let m: ChatMessage
    let p: ChatPicture
    @Binding var pull: CGFloat
    let close: () -> Void
    /// Which way this drag started: up or down takes the picture away, sideways is left to the page swipe.
    @State private var upDown: Bool?
    @State private var scale: CGFloat = 1
    @State private var settled: CGFloat = 1
    @State private var offset: CGSize = .zero
    @State private var rested: CGSize = .zero

    var body: some View {
        PictureImage(m: m, p: p, side: 1600, fill: false)
            .scaleEffect(scale).offset(offset)
            .offset(y: pull).scaleEffect(1 - min(abs(pull) / 1600, 0.12))
            .frame(maxWidth: .infinity, maxHeight: .infinity).contentShape(Rectangle())
            .gesture(MagnifyGesture().onChanged { g in scale = min(max(settled * g.magnification, 1), 6) }.onEnded { _ in
                settled = scale
                if scale == 1 { withAnimation(.snappy) { offset = .zero }; rested = .zero }
            })
            // zoomed in, a drag moves the picture. Not zoomed, a drag up or down takes it away and closes the
            // viewer; a sideways one belongs to the page swipe and is left alone
            .simultaneousGesture(DragGesture(minimumDistance: 12).onChanged { g in
                if scale > 1 {
                    offset = CGSize(width: rested.width + g.translation.width, height: rested.height + g.translation.height)
                } else {
                    if upDown == nil { upDown = abs(g.translation.height) > abs(g.translation.width) }
                    if upDown == true { pull = g.translation.height }
                }
            }.onEnded { g in
                if scale > 1 { rested = offset; return }
                let going = upDown == true && (abs(pull) > 110 || abs(g.predictedEndTranslation.height) > 500)
                upDown = nil
                if going { close() } else { withAnimation(.snappy) { pull = 0 } }
            })
            .onTapGesture(count: 2) {
                withAnimation(.snappy) {
                    scale = scale > 1 ? 1 : 2.5
                    offset = .zero
                }
                settled = scale
                rested = .zero
            }
            // one tap anywhere closes it; SwiftUI waits a moment first to see whether it is the first of two
            .onTapGesture { close() }
    }
}

/// The web links an agent wrote, as buttons: an artifact, a pull request, a preview. They open in the browser.
struct LinkRow: View {
    @Environment(\.openURL) private var openURL
    let m: ChatMessage

    /// What to call a link: "artifact" for one of Claude's, otherwise the site.
    static func label(_ url: URL) -> String {
        let host = (url.host ?? "link").replacingOccurrences(of: "www.", with: "")
        if host == "claude.ai", url.path.contains("/artifact") { return "artifact" }
        if host == "github.com", url.path.contains("/pull/") { return "pull request" }
        return host
    }

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(m.links.compactMap { URL(string: $0) }, id: \.absoluteString) { url in
                    Button { openURL(url) } label: {
                        let kind = LinkRow.label(url)
                        HStack(spacing: 6) {
                            // a pull request carries its number and the mark people know it by
                            if kind == "pull request" { Image(systemName: "arrow.triangle.pull").font(.system(size: 13, weight: .bold)) }
                            Text(kind == "pull request" ? "pull request #" + url.lastPathComponent : kind).font(.system(size: 15, weight: .semibold)).lineLimit(1)
                            Image(systemName: "arrow.up.right").font(.system(size: 12, weight: .bold))
                        }
                        // an artifact is purple and a pull request green, so each is found at a glance in the feed
                        .foregroundStyle(kind == "artifact" || kind == "pull request" ? Color.white : Palette.ink).padding(.horizontal, 14).frame(minHeight: 44)
                        .background(kind == "artifact" ? Palette.art : kind == "pull request" ? Color(red: 0.14, green: 0.53, blue: 0.21) : Palette.raised, in: Capsule())
                    }
                    .buttonStyle(.plain).accessibilityLabel("Open \(LinkRow.label(url))")
                }
            }
        }
    }
}
