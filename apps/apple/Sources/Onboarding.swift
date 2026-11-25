import SwiftUI

/// The first run. First the coding agent he uses most; then a screen for each job the core lists (what writes
/// his summaries, what listens, what speaks), each saying what it is for and letting him choose what does it,
/// with a choice already made that needs nothing from him. The last screen puts airband on his computer and
/// pairs the two. Nothing is downloaded until he has been through them.
///
/// The same picker is each job's Settings page (`Choices.swift`), so what he sees here is what he changes later.
struct OnboardingView: View {
    @EnvironmentObject var model: RadioModel
    @Environment(\.dismiss) private var dismiss
    // development only: `-airband.devStep 2` opens on that step, for a screenshot
    @State private var step = Dev.number("devStep")
    @State private var scanning = false
    /// What runs on this phone and is not on it yet, asked about once before it is downloaded.
    @State private var downloads: DownloadAsk?
    @State private var wrong: String?
    @State private var copied = false
    /// The keyboard is up.
    @State private var typing = false

    /// He asked for the pairing with no relay.
    // (development only: `-airband.devNoRelay YES` opens with it asked for, for a screenshot)
    @State private var noRelay = Dev.flag("devNoRelay")
    private var command: String { installCommand(relay: !noRelay) }
    /// The first step waits for him to pick the coding agent he uses: none is picked for him.
    private var unchosen: Bool { step == 0 && model.usedTools.isEmpty }
    /// The steps: the coding agent, each job, then the computer.
    private var jobs: [ChoiceJob] { model.jobs }
    private var last: Int { jobs.count + 1 }
    /// The computer step is skipped by someone who is paired already and is only being shown the choices.
    private var steps: Int { model.paired && step < last ? last : last + 1 }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                ForEach(0..<steps, id: \.self) { i in Capsule().fill(i <= step ? Palette.ink : Palette.raised).frame(height: 4) }
            }
            .padding(.top, 18).padding(.bottom, 20)
            ScrollViewReader { page in
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if step == 0 {
                        headline(Headline.tools)
                        ToolsPicker(single: true)
                    } else if step <= jobs.count {
                        let job = jobs[step - 1]
                        headline(job.headline)
                        label(job.label)
                        ChoicePicker(job: job, mode: .onboarding).id(step)
                    } else {
                        computer
                    }
                }
                .padding(.bottom, 16)
            }
            .scrollIndicators(.hidden)
            .environment(\.reveal) { id in withAnimation(.snappy(duration: 0.25)) { page.scrollTo(id, anchor: .top) } }
            }
            if let wrong { Text(wrong).font(.footnote).foregroundStyle(.red).padding(.bottom, 10) }
            if !typing {
            Button(action: next) {
                HStack(spacing: 10) {
                    if step == last { Image(systemName: "qrcode.viewfinder").font(.system(size: 19, weight: .semibold)) }
                    Text(step == last ? "Scan the code" : "Continue").font(.system(size: 18, weight: .semibold))
                }
                .foregroundStyle(Palette.ground).frame(maxWidth: .infinity, minHeight: 56)
                .background(unchosen ? Palette.dim : Palette.ink, in: RoundedRectangle(cornerRadius: 18))
            }
            .buttonStyle(.plain).disabled(unchosen)
            HStack {
                if step > 0 { Button("Back") { withAnimation(.snappy(duration: 0.2)) { step -= 1 } } }
                Spacer()
                if step == last {
                    Button("Paste a link instead") {
                        wrong = model.pairPasted()
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
        .darkPage()
        .interactiveDismissDisabled()
        .fullScreenCover(isPresented: $scanning) { ScanSheet().environmentObject(model) }
        // paired, by the scan or by a link: fetch what he chose and finish
        .onChange(of: model.paired) { _, paired in if paired && step == last { finish() } }
        .confirmationDialog(downloads?.title ?? "", isPresented: Binding(get: { downloads != nil }, set: { if !$0 { downloads = nil } }), titleVisibility: .visible) {
            Button(downloads?.button ?? "") { fetch() }
            // not now: each card in Settings offers its download
            Button("Not now", role: .cancel) { close(nil) }
        }
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
            Text("Paste this in a terminal on the computer where your agents work.").font(.system(size: 16)).foregroundStyle(Palette.soft)
            VStack(alignment: .leading, spacing: 14) {
                Text(command).font(.system(size: 14, weight: .medium, design: .monospaced)).foregroundStyle(Palette.signal).textSelection(.enabled)
                HStack(spacing: 10) {
                    Button {
                        UIPasteboard.general.string = command
                        copied = true
                    } label: { tile(copied ? "Copied" : "Copy") }
                    ShareLink(item: command) { tile("Send to your computer") }
                }
                .buttonStyle(.plain)
            }
            .padding(16).background(Palette.card, in: RoundedRectangle(cornerRadius: 16))
            .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(Palette.raised, lineWidth: 1))
            Text("It installs airband and shows a code to scan.").font(.system(size: 15)).foregroundStyle(Palette.soft)
            // the quiet second way: nothing of his passes through airband's relay
            VStack(alignment: .leading, spacing: 6) {
                Button(noRelay ? "Use the relay" : "Pair without the relay") {
                    withAnimation(.snappy(duration: 0.2)) { noRelay.toggle() }
                    copied = false
                }
                .font(.system(size: 15, weight: .medium)).foregroundStyle(Palette.quiet).buttonStyle(.plain)
                if noRelay {
                    Text("Your phone connects straight to your computer. You get no notifications this way.")
                        .font(.system(size: 14)).foregroundStyle(Palette.quiet).fixedSize(horizontal: false, vertical: true)
                }
            }
            .padding(.top, 4)
        }
    }
    private func tile(_ text: String) -> some View {
        Text(text).font(.system(size: 15, weight: .semibold)).foregroundStyle(Palette.ink).frame(maxWidth: .infinity, minHeight: 44).background(Palette.raised, in: RoundedRectangle(cornerRadius: 12))
    }

    private func next() {
        wrong = nil
        if step < last - 1 || (step == last - 1 && !model.paired) {
            withAnimation(.snappy(duration: 0.2)) { step += 1 }
        } else if step == last - 1 {
            finish()
        } else {
            scanning = true
        }
    }

    /// Ask once to download what he chose that runs on this phone, with its size, then close.
    private func finish() {
        if let ask = model.desk.downloadsWanted() { downloads = ask } else { fetch() }
    }

    /// Close, and fetch the models he chose that run on this phone, one after another, and get them ready, while
    /// he uses the app: getting the listening model ready takes up to 15 minutes the first time, and the
    /// microphone shows how far it has got. What goes wrong is told where he is then.
    private func fetch() {
        let model = self.model
        model.finishChoices { problem in if let problem { model.problem = problem } }
        close(nil)
    }

    /// A download that failed or was put off does not keep him out: what he chose stays chosen and its card
    /// says how it stands. What went wrong is told where he lands, since this closes.
    private func close(_ problem: String?) {
        UserDefaults.standard.set(true, forKey: "airband.onboarded")
        if let problem { model.problem = problem }
        dismiss()
    }
}
