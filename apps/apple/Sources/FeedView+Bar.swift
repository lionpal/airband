// The bottom of the chat: what he is about to say and to whom, the bar he types and talks in, and quick messages.
import SwiftUI

extension FeedView {
    // MARK: bottom

    /// What the agent that said this is called now: it may have been renamed since he picked the message.
    func name(of m: ChatMessage) -> String {
        model.view.feed.first { $0.id == m.id }?.from ?? m.from
    }
    private var placeholder: String {
        if let re = model.replyingTo { return "reply to \(name(of: re))" }
        if let only = model.onlyName { return "message \(only)" }
        if let room = model.view.rooms.first(where: { $0.id == model.room }) { return "message \(room.name)" }
        return "say anything"
    }
    /// Who the line above the buttons says his words will go to, when it says so: the message he picked or the
    /// agent he is looking at, while the microphone is open or a message is picked.
    private var aimed: String? {
        guard model.micOpen || model.replyingTo != nil else { return nil }
        return model.replyingTo.map { name(of: $0) } ?? model.onlyName
    }
    /// The agent it will go to, by id: the one that wrote the message he picked, or the one he is looking at.
    private var aimedAgent: String? {
        // a message he picked says who, even when that is nobody: the agent he is looking at does not stand in
        if let re = model.replyingTo { return re.agent.flatMap { $0.isEmpty ? nil : $0 } }
        return model.only
    }
    /// One quiet line above the buttons: who he is answering, or what the radio is doing.
    private var statusLine: String? {
        // who it will go to, whenever that is already settled: the message he picked, or the agent he is looking at
        let aim = model.replyingTo.map { name(of: $0) } ?? model.onlyName
        if model.micOpen { return aim.map { "to \($0) \u{b7} tap to send" } ?? "listening \u{b7} tap to send" }
        if let re = model.replyingTo { return "to \(name(of: re)) \u{b7} tap here to undo" }
        switch model.status {
        case .listening: return "listening \u{b7} tap to send"
        case .sending: return "sending"
        case .speaking(_, let callsign): return callsign ?? "airband"
        default: return nil
        }
    }

    var bar: some View {
        VStack(spacing: 10) {
            if let problem = model.problem {
                Button { model.problem = nil } label: { Text(problem).font(.footnote).foregroundStyle(.red).frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 12).padding(.vertical, 7).frosted(RoundedRectangle(cornerRadius: 12)) }
                    .buttonStyle(.plain).padding(.horizontal, 8)
            }
            if let ask = model.view.asking { AskCard(ask: ask) }
            if !model.tray.isEmpty { Tray(sends: !model.listening && !typingOpen) }
            if showQuick { quickGrid }
            if typingOpen {
                typingRow
            } else {
                talkingStatus
                buttonRow
            }
        }
        .padding(.top, 6).padding(.bottom, 6)
        // scrolled back: a thin bar at the very bottom takes him to the latest again
        .overlay(alignment: .bottom) {
            if !atEnd, !typingOpen, !model.view.feed.isEmpty {
                Button { toEnd += 1 } label: {
                    HStack(spacing: 5) {
                        Image(systemName: "arrow.down").font(.system(size: 10, weight: .bold))
                        Text("latest").font(.system(size: 12, weight: .semibold))
                    }
                    .foregroundStyle(Palette.soft).padding(.horizontal, 14).frame(height: 24).frosted(Capsule(), strength: 0.8)
                }
                .buttonStyle(.plain).offset(y: 14).transition(.opacity).accessibilityLabel("Go to the latest message")
            }
        }
        .animation(.easeOut(duration: 0.15), value: atEnd)
        .onChange(of: model.replyingTo?.id) { _, id in if id != nil && typingOpen { typing = true } }
    }

    /// The box he types in, with the way to attach and the way back to talking.
    private var typingRow: some View {
        HStack(spacing: 10) {
            // attach, or a quick message, without leaving the keyboard
            Button { withAnimation(.snappy(duration: 0.2)) { showQuick.toggle() } } label: {
                Image(systemName: "plus").font(.system(size: 20, weight: .semibold)).rotationEffect(.degrees(showQuick ? 45 : 0))
                    .foregroundStyle(showQuick ? Palette.ground : Palette.ink)
                    .frame(width: 44, height: 54).background(showQuick ? Palette.ink : Palette.field, in: RoundedRectangle(cornerRadius: 22))
            }
            .buttonStyle(.plain).accessibilityLabel("Attach, or quick messages")
            TextField("", text: $draft, prompt: Text(placeholder).foregroundColor(Palette.quiet), axis: .vertical)
                .lineLimit(1...4).focused($typing).font(.system(size: 17))
                .padding(.horizontal, 20).frame(minHeight: 54)
                .frosted(RoundedRectangle(cornerRadius: 27), strength: 0.8)
            let empty = draft.trimmingCharacters(in: .whitespaces).isEmpty
            Button {
                if empty {
                    typing = false
                    typingOpen = false
                } else {
                    model.send(draft)
                    draft = ""
                }
            } label: {
                Image(systemName: empty ? "mic.fill" : "arrow.up").font(.system(size: 22, weight: .bold))
                    .frame(width: 54, height: 54).background(empty ? Palette.raised : Palette.ink, in: Circle())
                    .foregroundStyle(empty ? Palette.ink : Palette.ground)
            }
            .buttonStyle(.plain).accessibilityLabel(empty ? "Back to talking" : "Send")
        }
        .padding(.horizontal, 8)
    }

    /// Above the buttons while he is not typing: what the voice is saying, what he is saying, or where his words go.
    @ViewBuilder private var talkingStatus: some View {
        if let caption = model.caption, model.hearing.isEmpty, !model.micOpen {
            // what the voice is saying, the words lighting up as it says them
            TimelineView(.periodic(from: .now, by: 0.06)) { _ in
                let said = min(model.captionReached(), caption.text.count)
                // a long message is followed along: the words just said, and the ones coming
                let from = max(0, said - 70)
                let shown = String(caption.text.dropFirst(from).prefix(140))
                (Text(shown.prefix(said - from)).foregroundColor(Palette.ink) + Text(shown.dropFirst(said - from)).foregroundColor(Palette.quiet))
                    .font(.system(size: 15, weight: .medium)).lineLimit(2).multilineTextAlignment(.center)
                    .frame(minHeight: 18).padding(.horizontal, 14).padding(.vertical, 4)
                    .frosted(RoundedRectangle(cornerRadius: 16), strength: 0.8).padding(.horizontal, 8).frame(maxWidth: .infinity)
            }
            .accessibilityHidden(true)
        } else if model.hearing.isEmpty {
            HStack(spacing: 6) {
                // whoever it will go to, with its badge, when that is settled
                if aimed != nil, let agent = aimedAgent, let badge = model.badge(of: agent) {
                    Badge(text: badge.0, colour: badge.1, size: 16)
                }
                Text(statusLine ?? " ").font(.system(size: 14, weight: .medium)).foregroundStyle(statusLine == nil ? Palette.quiet : Palette.soft)
            }
                .frame(height: 18).padding(.horizontal, 12).padding(.vertical, 4)
                .frosted(Capsule(), shown: statusLine != nil).accessibilityHidden(statusLine == nil)
                // a tap lets go of the message he picked: what he says next is placed afresh
                .onTapGesture { if !model.micOpen { model.replyingTo = nil } }
        } else {
            // his own words, as he says them: the end of a long sentence stays in view
            if let aim = model.replyingTo.map({ name(of: $0) }) ?? model.onlyName {
                HStack(spacing: 5) {
                    Text("to")
                    if let agent = aimedAgent, let badge = model.badge(of: agent) { Badge(text: badge.0, colour: badge.1, size: 16) }
                    Text(aim)
                }
                .font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.soft)
                .padding(.horizontal, 10).padding(.vertical, 3).frosted(Capsule())
            }
            Text("\u{201C}\(model.hearing)\u{201D}").font(.system(size: 17, weight: .medium)).foregroundStyle(Palette.ink)
                .lineLimit(2).truncationMode(.head).multilineTextAlignment(.center)
                .frame(minHeight: 18).padding(.horizontal, 14).padding(.vertical, 4)
                .frosted(RoundedRectangle(cornerRadius: 16), strength: 0.8).padding(.horizontal, 8).frame(maxWidth: .infinity)
                .animation(.easeOut(duration: 0.12), value: model.hearing)
                // a tap on his words closes the microphone and puts them in the box, to finish by hand
                .onTapGesture {
                    draft = model.cancelTalk()
                    typingOpen = true
                    typing = true
                }
                .accessibilityLabel("What you said. Tap to edit.").accessibilityAddTraits(.isButton)
        }
    }

    /// The buttons: attach or quick messages, reading aloud, talking, typing.
    private var buttonRow: some View {
        HStack {
            // +: attach a photo or a video, or send one of the things he says all the time
            // while he is talking it is the way out: a cross that closes the microphone and sends nothing
            Button {
                if model.listening { model.cancelTalk() } else { withAnimation(.snappy(duration: 0.2)) { showQuick.toggle() } }
            } label: { PlusButtonLabel(open: showQuick || model.listening) }
                .buttonStyle(.plain).accessibilityLabel(model.listening ? "Cancel" : "Attach, or quick messages").accessibilityAddTraits(showQuick ? .isSelected : [])
            Spacer(minLength: 0)
            // Join the channel: what arrives is read aloud, hands free. As large as the microphone beside
            // it, since talking and listening are the two things this is for. Green while it is on. It
            // keeps its place while he talks, dimmed: nothing is read over him.
            Button {
                let on = !model.calling
                Task { await model.setCalling(on) }
                joinedNote += 1
                withAnimation(.easeOut(duration: 0.2)) { joined = on }
            } label: {
                ZStack {
                    if model.calling { Circle().fill(Palette.signal).frame(width: 88, height: 88) } else { Circle().fill(.clear).frame(width: 88, height: 88).frosted(Circle()) }
                    Image(systemName: model.calling ? "speaker.wave.2.fill" : "speaker.slash.fill").font(.system(size: 30, weight: .semibold))
                        .foregroundStyle(model.calling ? Palette.ground : Palette.ink)
                }
                .frame(width: 96, height: 112)
            }
            .buttonStyle(.plain).disabled(model.listening).opacity(model.listening ? 0.35 : 1)
            .accessibilityLabel(model.calling ? "Stop reading aloud" : "Read aloud")
            .accessibilityAddTraits(model.calling ? .isSelected : [])
            Spacer(minLength: 0)
            Button { showQuick = false; Task { await model.talk() } } label: {
                ZStack {
                    Circle().fill(Palette.signal.opacity(model.listening ? 0.25 : 0)).frame(width: 104, height: 104)
                    Circle().fill(model.listening ? Palette.signal : Palette.ink).frame(width: 88, height: 88)
                    // open, it is the send button: nothing goes until he taps it
                    Image(systemName: model.listening ? "arrow.up" : "mic.fill").font(.system(size: 34, weight: .semibold)).foregroundStyle(Palette.ground)
                }
                .frame(width: 96, height: 112)
            }
            .buttonStyle(.plain).accessibilityLabel(model.listening ? "Send what you said" : "Talk")
                                Spacer(minLength: 0)
            HStack(spacing: 8) {
            if model.listening {
                // while he talks, this adds a photo or a video to what he is saying
                AttachButton()
            } else {
                Button { showQuick = false; typingOpen = true; typing = true } label: {
                    Image(systemName: "keyboard").font(.system(size: 20, weight: .semibold))
                        .frame(width: 56, height: 56).frosted(Circle())
                }
                .buttonStyle(.plain).accessibilityLabel("Type")
            }
            }
        }
        .padding(.horizontal, 18)
        // said once when he joins, then gone: what the green button now does
        .overlay(alignment: .top) {
            if joined {
                Text("New messages will be read aloud").font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.soft)
                    .padding(.horizontal, 14).frame(height: 28).frosted(Capsule(), strength: 0.9)
                    .offset(y: -30).transition(.opacity)
                    .task(id: joinedNote) {
                        try? await Task.sleep(nanoseconds: 3_000_000_000)
                        withAnimation(.easeOut(duration: 0.4)) { joined = false }
                    }
            }
        }
    }

    /// The quick messages, as a grid above the buttons. One tap sends one to the message he picked, or to whoever
    /// the tower works out. His own come after the standard ones.
    private var quickGrid: some View {
        let all = model.view.quick
        return VStack(spacing: 8) {
            RecentStrip()
            AttachTile { withAnimation(.snappy(duration: 0.2)) { showQuick = false } }
            // two to a row; an odd one out, usually one of his own, takes the whole row
            ForEach(Array(stride(from: 0, to: all.count, by: 2)), id: \.self) { i in
                HStack(spacing: 8) {
                    ForEach(all[i..<min(i + 2, all.count)], id: \.label) { q in
                        Button {
                            model.quick(q)
                            withAnimation(.snappy(duration: 0.2)) { showQuick = false }
                        } label: {
                            Text(q.label).font(.system(size: 16, weight: .semibold)).lineLimit(1).minimumScaleFactor(0.75)
                                .padding(.horizontal, 10).frame(maxWidth: .infinity, minHeight: 50)
                                .background(Palette.raised, in: RoundedRectangle(cornerRadius: 16))
                        }
                        .buttonStyle(.plain).accessibilityHint(q.text)
                    }
                }
            }
        }
        .padding(10)
        .background(Palette.card, in: RoundedRectangle(cornerRadius: 26))
        .padding(.horizontal, 8)
        .transition(.scale(scale: 0.92, anchor: .bottomLeading).combined(with: .opacity))
    }
}
