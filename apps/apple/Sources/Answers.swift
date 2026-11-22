import SwiftUI

// Answering: who unplaced words are for, the answers under a question, and everything behind one question.

/// His words could not be placed: who are they for? One tap sends them; the cross lets them go.
struct AskCard: View {
    @EnvironmentObject var model: RadioModel
    let ask: DeskAsk

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Who is this for?").font(.system(size: 16, weight: .semibold))
                    Text("\u{201C}\(ask.text)\u{201D}").font(.system(size: 14)).foregroundStyle(Palette.quiet).lineLimit(2)
                }
                Spacer(minLength: 8)
                Button { model.choose(nil) } label: {
                    RoundIcon("xmark", side: 32, glyph: 12, weight: .bold)
                }
                .buttonStyle(.plain).accessibilityLabel("Send it to nobody")
            }
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(ask.choices, id: \.id) { choice in
                        Button { model.choose(choice) } label: {
                            HStack(spacing: 7) {
                                Badge(text: choice.badge, colour: choice.colour, size: 20)
                                Text(choice.callsign).font(.system(size: 15, weight: .semibold)).lineLimit(1)
                            }
                            .padding(.horizontal, 12).frame(minHeight: 44).background(Palette.raised, in: Capsule())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
        .padding(14)
        .background(Palette.card, in: RoundedRectangle(cornerRadius: 22))
        .overlay(RoundedRectangle(cornerRadius: 22).strokeBorder(Palette.ask.opacity(0.5), lineWidth: 1))
        .padding(.horizontal, 8)
        .transition(.move(edge: .bottom).combined(with: .opacity))
    }
}
/// The answer row under a question: the agent's choices, then a small one that hands the decision back to the agent.
struct Answers: View {
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage
    var after: () -> Void = {}
    /// What the small button sends. The agent reads it as its answer.
    static let yolo = "Do whatever you think is best."

    /// Three answers, or a long one, do not fit side by side: each then gets a row of its own.
    private var stacked: Bool { buttons.count > 2 || buttons.contains { $0.0.count > 16 } }
    /// The buttons, each with what it stands for. A request for leave has two of its own, Allow and Deny; any
    /// other question has the answers it offered.
    private var buttons: [(String, Tapped)] {
        m.permission ? [(leaveWord(allow: true), .allow), (leaveWord(allow: false), .deny)] : m.options.enumerated().map { ($0.element, .option(index: UInt32($0.offset))) }
    }

    private func choice(_ i: Int) -> some View {
        let (text, tapped) = buttons[i]
        let marked = !m.permission && m.recommended == UInt32(i)
        return Button { model.tap(m, tapped); after() } label: {
            // a long answer wraps to two lines inside the button, then shrinks; it never touches the edges
            Text(text).font(.system(size: 17, weight: .semibold)).lineLimit(2).minimumScaleFactor(0.75)
                .multilineTextAlignment(.center).lineSpacing(-1).padding(.horizontal, 12).padding(.vertical, 6)
                .frame(maxWidth: .infinity, minHeight: 52)
                // green only for the answer the agent itself recommended; with no recommendation, none is marked
                .background(marked ? Palette.signal : Palette.raised, in: RoundedRectangle(cornerRadius: 16))
                .foregroundStyle(marked ? Palette.ground : Palette.ink)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(marked ? "\(text), recommended" : text)
    }

    private var yolo: some View {
        Button { model.answer(m, Answers.yolo); after() } label: {
            Text("YO\nLO").font(.system(size: 13, weight: .heavy, design: .rounded)).tracking(1).lineSpacing(-2)
                .multilineTextAlignment(.center).foregroundStyle(Palette.ink)
                .frame(width: 52, height: 52)
                .background(Palette.raised, in: RoundedRectangle(cornerRadius: 16))
        }
        .buttonStyle(.plain).accessibilityLabel("Do whatever you think is best")
    }

    /// Answer in his own words. It picks this question and opens the ordinary microphone at the bottom, which is
    /// where he sends from; nothing here changes while he talks. With no choices to tap it is the main button and
    /// says so; beside choices it is a small one. The keyboard beside it opens the text box for the same question.
    private func respond(wide: Bool) -> some View {
        HStack(spacing: 10) {
            Button {
                model.replyingTo = m
                after()
                Task { await model.talk() }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "mic.fill").font(.system(size: 17, weight: .semibold))
                    if wide { Text("Respond").font(.system(size: 17, weight: .semibold)) }
                }
                .foregroundStyle(wide ? Palette.ground : Palette.ink)
                .frame(maxWidth: wide ? .infinity : nil, minHeight: 52).frame(width: wide ? nil : 52)
                .background(wide ? Palette.ink : Palette.raised, in: RoundedRectangle(cornerRadius: 16))
            }
            .buttonStyle(.plain).accessibilityLabel("Answer by voice")
            Button {
                model.replyingTo = m
                after()
                model.typeNow = true
            } label: {
                Image(systemName: "keyboard").font(.system(size: 16, weight: .semibold)).foregroundStyle(Palette.ink)
                    .frame(width: 52, height: 52).background(Palette.raised, in: RoundedRectangle(cornerRadius: 16))
            }
            .buttonStyle(.plain).accessibilityLabel("Type an answer")
        }
    }

    /// The agent that asked, when it has said it takes this answer only in its own session and the computer
    /// does not run it: an answer from here would reach it as a passed-on message, and it would ask again.
    private var apart: DeskMember? {
        guard m.ownSession else { return nil }
        return m.agent.flatMap { model.member($0)?.agent }.flatMap { $0.held ? nil : $0 }
    }
    @State private var taking: DeskMember?

    /// In place of the answers: why one from here will not do, and the way to make it do.
    private func takeOver(_ agent: DeskMember) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(agent.terminal ? "It wants this answer at its own keyboard. It was started in a terminal on your computer, so anything you send from your phone reaches it as a note left for it." : "It wants this answer at its own keyboard. It was started outside airband on your computer, so anything you send from your phone reaches it as a note left for it.")
                .font(.system(size: 14)).foregroundStyle(Palette.soft).fixedSize(horizontal: false, vertical: true)
            Button { taking = agent } label: {
                Text("Take over, then answer").font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.ground)
                    .frame(maxWidth: .infinity, minHeight: 52).background(Palette.ink, in: RoundedRectangle(cornerRadius: 16))
            }
            .buttonStyle(.plain)
        }
        .takeOverAlert($taking)
    }

    var body: some View {
        if let agent = apart {
            takeOver(agent)
        } else if stacked {
            VStack(spacing: 10) {
                ForEach(buttons.indices.dropLast(), id: \.self) { i in choice(i) }
                HStack(spacing: 10) {
                    choice(buttons.count - 1)
                    if !m.permission { respond(wide: false); yolo }
                }
            }
        } else {
            HStack(spacing: 10) {
                ForEach(buttons.indices, id: \.self) { i in choice(i) }
                // permission to do something on his computer is a yes or a no: it is never handed back to the agent
                if !m.permission { respond(wide: buttons.isEmpty); yolo }
            }
        }
    }
}
/// Everything behind one question: what he and the agent said just before, exactly what the agent wrote, and the
/// same answers, or his own words.
struct ContextView: View {
    @EnvironmentObject var model: RadioModel
    @Environment(\.dismiss) private var dismiss
    let m: ChatMessage

    /// The last few things between him and this agent before the question.
    private var before: [ChatMessage] {
        model.thread(before: m)
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text(m.fromAndStation).font(.system(size: 20, weight: .bold))
                Spacer()
                Button { dismiss() } label: {
                    RoundIcon("xmark", side: 34, glyph: 14, weight: .bold)
                }
                .buttonStyle(.plain).accessibilityLabel("Close")
            }
            .padding(.horizontal, 20).padding(.top, 20).padding(.bottom, 14)
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        ForEach(before, id: \.id) { b in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(b.from).font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.quiet)
                                Text(b.text).font(.system(size: 16)).lineSpacing(3).foregroundStyle(Palette.soft)
                            }
                        }
                        if !m.pictures.isEmpty { PictureStrip(m: m, height: 200) }
                        if !m.links.isEmpty { LinkRow(m: m) }
                        Prose(text: model.whole(m), size: 17, colour: Palette.ink).textSelection(.enabled)
                            .padding(16).background(Palette.card, in: RoundedRectangle(cornerRadius: 22)).id("question")
                    }
                    .padding(.horizontal, 16).padding(.bottom, 12)
                }
                .onAppear { proxy.scrollTo("question", anchor: .top) }
            }
            VStack(spacing: 10) {
                // the answers, and the microphone and keyboard for one of his own: both close this and open the
                // ordinary one at the bottom of the chat, aimed at this question
                Answers(m: m, after: { dismiss() })
            }
            .padding(.horizontal, 16).padding(.top, 10).padding(.bottom, 12)
        }
        .darkPage()
        .onAppear { model.loadFull(m) }
    }
}
