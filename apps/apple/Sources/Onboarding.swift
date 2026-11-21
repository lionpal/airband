import SwiftUI

/// The first run. A screen for each thing airband does for him, each saying what it is for and then letting him
/// choose what does it, with a choice already made that needs nothing from him. The last screen puts airband on
/// his computer and pairs the two. Nothing is downloaded until he has been through them.
///
/// The same pickers are the Settings pages (`Choices.swift`), so what he sees here is what he changes later.
struct OnboardingView: View {
    @EnvironmentObject var model: RadioModel
    @Environment(\.dismiss) private var dismiss
    // development only: `-airband.devStep 2` opens on that step, for a screenshot
    @State private var step = UserDefaults.standard.integer(forKey: "airband.devStep")
    @State private var scanning = false
    @State private var finishing = false
    @State private var working: String?
    @State private var progress: Double?
    @State private var wrong: String?
    @State private var copied = false
    /// The keyboard is up.
    @State private var typing = false
    private let tick = Timer.publish(every: 0.25, on: .main, in: .common).autoconnect()

    private static let command = "brew install lionpal/airband/airband && airband start"
    /// The computer step is skipped by someone who is paired already and is only being shown the choices.
    private var steps: Int { model.paired && step < 4 ? 4 : 5 }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                ForEach(0..<steps, id: \.self) { i in Capsule().fill(i <= step ? Palette.ink : Palette.raised).frame(height: 4) }
            }
            .padding(.top, 18).padding(.bottom, 20)
            ScrollViewReader { page in
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    switch step {
                    case 0:
                        headline(Headline.tools)
                        Text("Choose the one you use most. You can add more later.").font(.system(size: 16)).foregroundStyle(Palette.soft)
                        ToolsPicker(single: true)
                    case 1:
                        headline(Headline.reads)
                        label("Model that writes the summaries")
                        ReadsPicker(mode: .onboarding)
                    case 2:
                        headline(Headline.ears)
                        label("Model that writes down what you say")
                        EarsPicker(mode: .onboarding)
                    case 3:
                        headline(Headline.voice)
                        Text("Hear each agent\u{2019}s news read out, even with the screen off.").font(.system(size: 16)).foregroundStyle(Palette.soft)
                        label("Model that speaks")
                        VoicePicker(mode: .onboarding)
                    default:
                        computer
                    }
                }
                .padding(.bottom, 16)
            }
            .scrollIndicators(.hidden)
            .environment(\.reveal) { id in withAnimation(.snappy(duration: 0.25)) { page.scrollTo(id, anchor: .top) } }
            }
            if let working {
                VStack(alignment: .leading, spacing: 6) {
                    if let progress { ProgressView(value: progress).tint(Palette.ink) } else { ProgressView().controlSize(.small) }
                    Text(working).font(.system(size: 14)).foregroundStyle(Palette.soft)
                }
                .padding(.bottom, 12)
            }
            if let wrong { Text(wrong).font(.footnote).foregroundStyle(.red).padding(.bottom, 10) }
            if !typing {
            Button(action: next) {
                HStack(spacing: 10) {
                    if step == 4 { Image(systemName: "qrcode.viewfinder").font(.system(size: 19, weight: .semibold)) }
                    Text(step == 4 ? "Scan the code" : "Continue").font(.system(size: 18, weight: .semibold))
                }
                .foregroundStyle(Palette.ground).frame(maxWidth: .infinity, minHeight: 56)
                .background(finishing ? Palette.dim : Palette.ink, in: RoundedRectangle(cornerRadius: 18))
            }
            .buttonStyle(.plain).disabled(finishing)
            HStack {
                if step > 0 && !finishing { Button("Back") { withAnimation(.snappy(duration: 0.2)) { step -= 1 } } }
                Spacer()
                if step == 4 && !finishing {
                    Button("Paste a link instead") {
                        model.pair(with: UIPasteboard.general.string ?? "")
                        wrong = model.problem
                    }
                }
            }
            .font(.system(size: 15, weight: .medium)).foregroundStyle(Palette.quiet).padding(.top, 12).frame(minHeight: 30)
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { _ in typing = true }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillHideNotification)) { _ in typing = false }
        .padding(.horizontal, 24).padding(.bottom, 16)
        .frame(maxWidth: 560).frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Palette.ground.ignoresSafeArea())
        .preferredColorScheme(.dark)
        .interactiveDismissDisabled()
        .fullScreenCover(isPresented: $scanning) { ScanSheet().environmentObject(model) }
        // paired, by the scan or by a link: fetch what he chose and finish
        .onChange(of: model.paired) { _, paired in if paired && step == 4 { finish() } }
        .onReceive(tick) { _ in if finishing, progress != nil { progress = Double(localModelProgress()) / 100 } }
    }

    private func headline(_ text: String) -> some View {
        Text(text).font(.system(size: 30, weight: .bold)).tracking(-0.6).foregroundStyle(Palette.ink).fixedSize(horizontal: false, vertical: true)
    }
    private func label(_ text: String) -> some View {
        Text(text).font(.system(size: 14, weight: .semibold)).foregroundStyle(Palette.quiet).padding(.top, 2)
    }

    /// The last step: put airband on his computer, then scan the code it shows.
    private var computer: some View {
        VStack(alignment: .leading, spacing: 18) {
            headline("Last, your computer.")
            Text("airband runs on the Mac where your agents work. Open Terminal there and paste this.").font(.system(size: 16)).foregroundStyle(Palette.soft)
            VStack(alignment: .leading, spacing: 14) {
                Text(Self.command).font(.system(size: 14, weight: .medium, design: .monospaced)).foregroundStyle(Palette.signal).textSelection(.enabled)
                HStack(spacing: 10) {
                    Button {
                        UIPasteboard.general.string = Self.command
                        copied = true
                    } label: { tile(copied ? "Copied" : "Copy") }
                    ShareLink(item: Self.command) { tile("AirDrop or share") }
                }
                .buttonStyle(.plain)
            }
            .padding(16).background(Palette.card, in: RoundedRectangle(cornerRadius: 16))
            .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(Palette.raised, lineWidth: 1))
            Text("It installs airband, keeps it running after a restart, and shows a code on the screen.").font(.system(size: 15)).foregroundStyle(Palette.soft)
        }
    }
    private func tile(_ text: String) -> some View {
        Text(text).font(.system(size: 15, weight: .semibold)).foregroundStyle(Palette.ink).frame(maxWidth: .infinity, minHeight: 44).background(Palette.raised, in: RoundedRectangle(cornerRadius: 12))
    }

    private func next() {
        wrong = nil
        // Leaving the summaries step with nothing picked: what is shown as chosen there, the sign-in of the
        // coding tool he uses most, is what his computer is asked to use once it is paired.
        if step == 1, model.pendingReads == nil, model.view.reads.isEmpty, let usual = readingServices().first(where: { $0.login == model.tools.first }) {
            model.chooseReads(usual.kind, key: nil, settings: [:]) { _ in }
        }
        if step < 3 || (step == 3 && !model.paired) {
            withAnimation(.snappy(duration: 0.2)) { step += 1 }
        } else if step == 3 {
            finish()
        } else {
            scanning = true
        }
    }

    /// Fetch the models he chose that run on this phone, one after another, then close.
    private func finish() {
        finishing = true
        model.fetchChosen { label, fraction in
            (working, progress) = (label, fraction)
        } done: { problem in
            (finishing, working, progress, wrong) = (false, nil, nil, problem)
            // a download that failed does not keep him out: it can be tried again in Settings
            UserDefaults.standard.set(true, forKey: "airband.onboarded")
            dismiss()
        }
    }
}
