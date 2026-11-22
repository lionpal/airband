import AVFoundation
import Photos
import PhotosUI
import SwiftUI
import UIKit

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
