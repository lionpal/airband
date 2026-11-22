import SwiftUI

// One message in the chat, each kind drawn its own way: a report, a question, something he said, and
// something of his that has not gone yet.

/// Who said a message, as the small line above it. Tap it to see only that agent.
private struct Speaker: View {
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage

    var body: some View {
        Button { model.look(agent: m.agent) } label: {
            HStack(spacing: 7) {
                if !m.badge.isEmpty { Badge(text: m.badge, colour: m.colour, size: 20) }
                Text(m.station.isEmpty ? m.from : "\(m.from) · \(m.station)").font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.quiet)
                // it was to be read to him while he was listening, and was not: a small mark. Why is one
                // long press away, in the line's menu.
                // no model read it: it is as the agent wrote it, and a small mark says so
                if !m.unjudged.isEmpty {
                    Image(systemName: "text.badge.xmark").font(.system(size: 11, weight: .semibold)).foregroundStyle(Palette.dim)
                        .accessibilityLabel(m.unjudged)
                }
                if !m.notRead.isEmpty {
                    Image(systemName: "speaker.slash.fill").font(.system(size: 11, weight: .semibold)).foregroundStyle(Palette.dim)
                        .accessibilityLabel(m.notRead)
                }
            }
        }
        .buttonStyle(.plain).accessibilityLabel("\(m.from). Show only this agent.")
    }
}
/// Something an agent reported: who, one line, and any pictures and links that came with it. Tap it to answer
/// that agent and to see everything it wrote.
struct Report: View {
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage
    /// He turned this one the other way from his usual: opened it when he reads the short form, or closed it
    /// when he reads everything.
    @State private var expanded = false
    private var picked: Bool { model.replyingTo?.id == m.id }
    /// Showing everything it wrote, and not the short form.
    private var whole: Bool { expanded != model.showWhole }

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 8) {
                Speaker(m: m)
                if m.stalled { Text("stopped").font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.quiet).padding(.horizontal, 8).padding(.vertical, 2).background(Palette.raised, in: Capsule()) }
            }
            // a tap picks it as the message he is answering, and nothing else: it does not change size under his
            // finger. Seeing all of what it wrote is the small button beside it.
            HStack(alignment: .bottom, spacing: 6) {
                Button { model.replyingTo = picked ? nil : m } label: {
                    Prose(text: whole ? model.whole(m) : m.text, plain: !whole)
                }
                .buttonStyle(.plain)
                .accessibilityHint(picked ? "Stop answering this one" : "Answer this agent: what you say or type next goes to it")
                if m.long {
                    Button {
                        expanded.toggle()
                        if whole { model.loadFull(m) }
                    } label: {
                        Image(systemName: whole ? "chevron.up" : "chevron.down").font(.system(size: 11, weight: .bold)).foregroundStyle(Palette.quiet)
                            .frame(width: 30, height: 30).background(Palette.field, in: Circle())
                    }
                    .buttonStyle(.plain).accessibilityLabel(whole ? "Show the short version" : "Show everything it wrote")
                }
            }
            // when he reads everything by default, the whole of it is fetched as the line comes on screen
            .onAppear { if whole { model.loadFull(m) } }
            .onChange(of: model.showWhole) { _, _ in if whole { model.loadFull(m) } }
            if !m.pictures.isEmpty { PictureStrip(m: m, height: 150).padding(.top, 3) }
            if !m.links.isEmpty { LinkRow(m: m).padding(.top, 3) }
        }
        .padding(.horizontal, 6).padding(.vertical, picked ? 8 : 0)
        .background(picked ? Palette.card : .clear, in: RoundedRectangle(cornerRadius: 16))
        .padding(.horizontal, picked ? -4 : 0)
        .contextMenu { LineMenu(m: m) }
    }
}
/// What a long press on something an agent said offers: its words to copy and, for one that was not read
/// aloud while he was listening, why.
private struct LineMenu: View {
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage

    var body: some View {
        if !m.unjudged.isEmpty || !m.notRead.isEmpty {
            Section([m.unjudged, m.notRead].filter { !$0.isEmpty }.joined(separator: " ")) { copy }
        } else {
            copy
        }
    }
    private var copy: some View {
        Button { UIPasteboard.general.string = model.whole(m) } label: { Label("Copy", systemImage: "doc.on.doc") }
    }
}
/// A question. While it is open it has an amber edge and its answers as buttons; once answered it folds down to
/// the question alone. Tap the sentence to pick it: what he says or types next is then its answer. The button in
/// the corner opens everything behind the question.
struct Question: View {
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage
    @State private var showContext = false
    private var picked: Bool { model.replyingTo?.id == m.id }

    private var text: AttributedString {
        (try? AttributedString(markdown: m.text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(m.text)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: m.open ? 12 : 6) {
            Speaker(m: m)
            Button { if m.open { model.replyingTo = picked ? nil : m } else { showContext = true } } label: {
                Text(text)
                    .font(.system(size: m.open ? 20 : 16, weight: m.open ? .semibold : .regular)).tracking(m.open ? -0.3 : 0)
                    .foregroundStyle(m.open ? Palette.ink : Palette.soft)
                    .lineSpacing(2).multilineTextAlignment(.leading).frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.trailing, m.open ? 72 : 0)
            }
            .buttonStyle(.plain).accessibilityHint(m.open ? (picked ? "Stop answering this one" : "Answer this one: what you say or type next goes to it") : "Show what was behind this question")
            .accessibilityAddTraits(picked ? .isSelected : [])
            if !m.pictures.isEmpty { PictureStrip(m: m, height: m.open ? 150 : 96) }
            if !m.links.isEmpty { LinkRow(m: m) }
            if m.open { Answers(m: m) }
        }
        .padding(.horizontal, 16).padding(.top, 14).padding(.bottom, m.open ? 14 : 12)
        .background(Palette.card, in: RoundedRectangle(cornerRadius: 24))
        // waiting on him: an amber edge. Picked: a white one, and what he says next is its answer.
        .overlay(RoundedRectangle(cornerRadius: 24).strokeBorder(picked ? Palette.ink : Palette.ask.opacity(0.45), lineWidth: picked ? 2 : (m.open ? 1 : 0)))
        .overlay(alignment: .topTrailing) {
            if m.open {
                // more about it, and to its right the way to let it go unanswered. Each has its own touch area,
                // with a gap between, so one is not hit for the other
                HStack(spacing: 6) {
                    Button { showContext = true } label: {
                        RoundIcon("info", side: 28, glyph: 13, weight: .bold).foregroundStyle(Palette.soft)
                            .frame(width: 38, height: 44)
                    }
                    .buttonStyle(.plain).accessibilityLabel("More about this question")
                    Button { model.dismiss(m) } label: {
                        RoundIcon("xmark", side: 28, glyph: 12, weight: .bold).foregroundStyle(Palette.soft)
                            .frame(width: 38, height: 44)
                    }
                    .buttonStyle(.plain).accessibilityLabel("Ignore this question")
                }
                .padding(.trailing, 4)
            }
        }
        .contextMenu { LineMenu(m: m) }
        .sheet(isPresented: $showContext) { ContextView(m: m).environmentObject(model) }
        #if DEBUG
        // development only: `-airband.devContext <callsign>` opens that agent's question, for a screenshot,
        // and `-airband.devPick <callsign>` picks it, as a tap on the sentence would
        .onAppear {
            if UserDefaults.standard.string(forKey: "airband.devContext") == m.from { showContext = true }
            if UserDefaults.standard.string(forKey: "airband.devPick") == m.from, m.open { model.replyingTo = m }
        }
        #endif
    }
}
/// Something he said: on the right, with who it went to.
struct Mine: View {
    @EnvironmentObject var model: RadioModel
    let m: ChatMessage
    /// The last thing he said: the only one that says whether it was delivered and read.
    var latest = false
    /// A long one shows its first lines; a tap opens it whole, and another closes it.
    @State private var whole = false
    private var long: Bool { m.long || m.text.count > 140 || m.text.filter { $0 == "\n" }.count > 4 }

    var body: some View {
        VStack(alignment: .trailing, spacing: 3) {
            if !m.to.isEmpty {
                // who it went to, with that agent's badge when it went to one. A tap shows only that agent, so
                // what he says next goes to it
                Button { if m.toAgents.count == 1 { model.look(agent: m.toAgents[0]) } } label: {
                    HStack(spacing: 5) {
                        Text("to").font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.quiet)
                        if !m.badge.isEmpty { Badge(text: m.badge, colour: m.colour, size: 16) }
                        Text(m.to.joined(separator: ", ")).font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.quiet).lineLimit(1)
                    }
                    .frame(minHeight: 26).contentShape(Rectangle())
                }
                .buttonStyle(.plain).accessibilityLabel("To \(m.to.joined(separator: ", ")). Show only this agent.")
            }
            if m.queued {
                // it waits until the agent stops what it is doing: until then he can take it back
                Button { model.unsend(m) } label: {
                    HStack(spacing: 5) {
                        Text("waiting its turn").foregroundStyle(Palette.quiet)
                        Text("\u{b7} take back").foregroundStyle(Palette.soft)
                    }
                    .font(.system(size: 12, weight: .semibold)).frame(minHeight: 28)
                }
                .buttonStyle(.plain).accessibilityLabel("Waiting its turn. Take it back.")
            }
            // One upright picture (a screenshot) stands to the left of his words, so it costs no height of its
            // own. Anything else he attached sits over his words, its left edge on theirs.
            let beside = m.pictures.count == 1 && m.pictures[0].height > m.pictures[0].width
            HStack(alignment: .top, spacing: 8) {
            if beside { PictureStrip(m: m, height: 104, still: true) }
            VStack(alignment: .leading, spacing: 6) {
            if !m.pictures.isEmpty, !beside { PictureStrip(m: m, height: 120, still: true) }
            VStack(alignment: .trailing, spacing: 4) {
                Text(whole ? model.whole(m) : m.text).font(.system(size: 16, weight: .medium)).foregroundStyle(Palette.ground)
                    .lineLimit(whole ? nil : 5)
                if long {
                    Image(systemName: whole ? "chevron.up" : "chevron.down").font(.system(size: 11, weight: .bold)).foregroundStyle(Palette.ground.opacity(0.45))
                }
            }
            .padding(.horizontal, 14).padding(.vertical, 9)
            .background(Palette.ink, in: UnevenRoundedRectangle(topLeadingRadius: 18, bottomLeadingRadius: 18, bottomTrailingRadius: 6, topTrailingRadius: 18))
            .opacity(m.queued ? 0.6 : 1)
            .contentShape(Rectangle())
            // hold it: copy it, or send the same words to someone else
            .contextMenu {
                Button { UIPasteboard.general.string = model.whole(m) } label: { Label("Copy", systemImage: "doc.on.doc") }
                Menu {
                    ForEach(model.agents, id: \.id) { agent in
                        Button(agent.callsign) { model.resend(m, to: agent) }
                    }
                } label: { Label("Send to someone else", systemImage: "arrowshape.turn.up.right") }
            }
            .onTapGesture {
                guard long else { return }
                // what the chat carries of a very long one is its opening: the rest is fetched when he opens it
                if !whole && m.long { model.loadFull(m) }
                withAnimation(.snappy(duration: 0.2)) { whole.toggle() }
            }
            }
            }
            // has it got there, and has the agent read it: one tick when it is delivered, two and the time once
            // the agent has taken it in (an agent busy with something else may not for a long while)
            if latest, !m.queued, !m.to.isEmpty {
                HStack(spacing: 4) {
                    Image(systemName: m.seen > 0 ? "checkmark.circle.fill" : "checkmark").font(.system(size: 10, weight: .bold))
                    Text(m.seen > 0 ? "read " + Date(timeIntervalSince1970: Double(m.seen) / 1000).formatted(date: .omitted, time: .shortened) : "delivered")
                        .font(.system(size: 11, weight: .medium))
                }
                .foregroundStyle(m.seen > 0 ? Palette.signal.opacity(0.85) : Palette.dim)
                .accessibilityLabel(m.seen > 0 ? "Read" : "Delivered, not read yet")
            }
        }
        .frame(maxWidth: .infinity, alignment: .trailing).padding(.leading, 50)
        .accessibilityElement(children: .combine).accessibilityLabel("You, to \(m.to.joined(separator: ", ")): \(m.text)")
        .accessibilityAddTraits(long ? .isButton : [])
    }
}
/// Something he sent that has not reached his computer. Once it has been trying for a while it says how the
/// last try went, each way it was tried. One that was given up on says why and stays, to send again or delete.
struct Unsent: View {
    @EnvironmentObject var model: RadioModel
    let o: Outgoing

    /// Said only once it has been trying for ten seconds: before that it is simply on its way.
    private var trying: String? {
        guard o.failed == nil, !o.trail.isEmpty, o.lastTried >= o.at + 10_000 else { return nil }
        return "Trying. " + o.trail.joined(separator: " ") + (o.tries > 1 ? " \(o.tries) tries." : "")
    }

    var body: some View {
        VStack(alignment: .trailing, spacing: 5) {
            Text(o.label ?? o.text).font(.system(size: 16, weight: .medium)).foregroundStyle(Palette.ground)
                .padding(.horizontal, 14).padding(.vertical, 9)
                .background(Palette.ink.opacity(o.failed == nil ? 0.45 : 0.3), in: RoundedRectangle(cornerRadius: 18))
            if let failed = o.failed {
                Text("Not sent. " + failed).font(.system(size: 13)).foregroundStyle(Palette.ask).multilineTextAlignment(.trailing)
                HStack(spacing: 18) {
                    Button("Delete") { model.discardUnsent(o) }.foregroundStyle(Palette.quiet)
                    Button("Send again") { model.sendAgain(o) }.foregroundStyle(Palette.ink)
                }
                .font(.system(size: 14, weight: .semibold)).buttonStyle(.plain)
            } else if o.stage == .left {
                Text("With the relay. Your computer gets it when it is back.").font(.system(size: 13)).foregroundStyle(Palette.quiet).multilineTextAlignment(.trailing)
            } else if let trying {
                Text(trying).font(.system(size: 13)).foregroundStyle(Palette.quiet).multilineTextAlignment(.trailing)
            }
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(o.failed == nil ? "Sending: \(o.label ?? o.text)" : "Not sent: \(o.label ?? o.text)")
    }
}
