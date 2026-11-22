import CarPlay
import SwiftUI
import Combine
import UIKit

/// The car's screen. A list with one thing on it, "Talk to your agents", and while a conversation is on, the
/// voice screen: a few words on what is happening and buttons to talk, to stop what is being read, and to end.
///
/// Apple's rules for this kind of app shape it. Nothing an agent says is shown, only heard. The microphone is
/// used only while the voice screen is up, and the audio is held only for as long as the conversation is on.
/// Nothing here asks him to pick up his phone.
final class CarPlaySceneDelegate: UIResponder, CPTemplateApplicationSceneDelegate {
    private var car: CPInterfaceController?
    private var voiceScreen: CPVoiceControlTemplate?
    /// The button on the voice screen that names who is speaking.
    private var speaker: CPBarButton?
    private var speakerFace: CPBarButton?
    private var watching: Set<AnyCancellable> = []
    private var model: RadioModel { RadioModel.shared }
    /// The conversation was started from the car: it ends when the car goes.
    private var started = false

    func templateApplicationScene(_ scene: CPTemplateApplicationScene, didConnect controller: CPInterfaceController) {
        car = controller
        model.voice.inCar = true
        controller.setRootTemplate(root(), animated: false, completion: nil)
        // what the list says follows whether he is paired; what the voice screen says follows the conversation
        model.$view.map { view in [view.paired ? "paired" : ""] + view.rooms.flatMap(\.members).map { "\($0.callsign):\($0.state)" } }.removeDuplicates().receive(on: DispatchQueue.main).sink { [weak self] _ in self?.refreshRoot() }.store(in: &watching)
        model.$status.combineLatest(model.$micOpen).receive(on: DispatchQueue.main).sink { [weak self] _ in self?.show() }.store(in: &watching)
        model.$calling.removeDuplicates().receive(on: DispatchQueue.main).sink { [weak self] on in if !on { self?.close() } }.store(in: &watching)
        // a conversation already on when the car connects carries on, on the car's screen
        if model.calling { open() }
    }

    func templateApplicationScene(_ scene: CPTemplateApplicationScene, didDisconnectInterfaceController controller: CPInterfaceController) {
        watching.removeAll()
        if started && model.calling { Task { @MainActor in await self.model.setCalling(false) } }
        started = false
        model.voice.inCar = false
        (car, voiceScreen) = (nil, nil)
    }

    /// The Talk tile on the car's dashboard, or a banner, was tapped: go straight into the conversation with
    /// the microphone open, for the agent the banner was about if there was one.
    func scene(_ scene: UIScene, openURLContexts contexts: Set<UIOpenURLContext>) {
        guard let url = contexts.first?.url, let route = OpenWith.route(url), model.view.paired else { return }
        if case .answer(let agent?) = route {
            start(then: agent)
        } else {
            model.only = nil
            start(then: nil)
            // already routed: the Talk tile, or a banner about no one in particular
            Task { @MainActor in
                guard self.model.calling else { return }
                if case .talk = route { self.model.replyingTo = nil }
                self.model.openMic()
            }
        }
    }

    // MARK: the list

    private func root() -> CPListTemplate {
        guard model.view.paired else {
            let item = CPListItem(text: "Not paired with a computer yet", detailText: nil)
            item.isEnabled = false
            return CPListTemplate(title: "airband", sections: [CPListSection(items: [item])])
        }
        let everyone = CPListItem(text: "Talk to everyone", detailText: nil, image: UIImage(systemName: "mic.fill"))
        everyone.handler = { [weak self] _, done in
            // through look(), like every change of who he is looking at: a message picked on the phone is let go
            self?.model.look()
            self?.start(then: nil)
            done()
        }
        var sections = [CPListSection(items: [everyone])]
        let agents = model.agents
        // Who is waiting on him: choosing one has its question read out, then opens the microphone for the
        // answer. What it asked is heard, not shown.
        let waiting = agents.filter { $0.state == .asking }.map { agent -> CPListItem in
            let item = CPListItem(text: agent.callsign, detailText: "Waiting on you", image: avatar(of: agent.id, side: 44))
            item.handler = { [weak self] _, done in
                self?.start(then: agent.id)
                done()
            }
            return item
        }
        if !waiting.isEmpty { sections.append(CPListSection(items: waiting, header: "Waiting on you", sectionIndexTitle: nil)) }
        let working = agents.filter { $0.state == .working }.map { agent -> CPListItem in
            let item = CPListItem(text: agent.callsign, detailText: "Working", image: avatar(of: agent.id, side: 44))
            item.handler = { [weak self] _, done in
                self?.model.look(room: self?.model.room, agent: agent.id)
                self?.start(then: nil)
                done()
            }
            return item
        }
        if !working.isEmpty { sections.append(CPListSection(items: working, header: "Working", sectionIndexTitle: nil)) }
        return CPListTemplate(title: "airband", sections: sections)
    }

    private func refreshRoot() {
        guard voiceScreen == nil else { return }
        car?.setRootTemplate(root(), animated: false, completion: nil)
    }

    // MARK: the conversation

    /// Start the conversation and put up the voice screen. With an agent named, its question is read out
    /// first and the microphone then opens for the answer.
    private func start(then answer: String?) {
        Task { @MainActor in
            if let answer, let team = self.model.member(answer)?.team {
                self.model.room = team.id
                self.model.only = answer
            }
            if !self.model.calling { await self.model.setCalling(true) }
            guard self.model.calling else {
                // the microphone is not allowed. It can only be allowed on the phone, which he is not asked to touch
                let ok = CPAlertAction(title: "OK", style: .cancel) { [weak self] _ in self?.car?.dismissTemplate(animated: true, completion: nil) }
                self.car?.presentTemplate(CPAlertTemplate(titleVariants: ["airband is not allowed to use the microphone", "Microphone not allowed"], actions: [ok]), animated: true, completion: nil)
                self.model.problem = nil
                return
            }
            self.started = true
            self.open()
            if let answer { self.model.answer(answer) }
        }
    }

    /// Put up the voice screen for the conversation that is on.
    private func open() {
        guard voiceScreen == nil, let car else { return }
        // The words of a state cannot change once the screen is made, so they are made for this conversation:
        // with one agent when he is answering one, with all of them otherwise. Longest first; the car shows
        // the longest that fits.
        let with = model.onlyName
        let screen = CPVoiceControlTemplate(voiceControlStates: [
            state(.idle, with.map { ["Waiting for \($0)", "Waiting"] } ?? ["Waiting for your agents", "Waiting"], "ellipsis", talk: ("Talk", "mic.fill")),
            state(.listening, with.map { ["Listening, for \($0)", "Listening"] } ?? ["Listening"], "mic.fill", talk: ("Send", "arrow.up"), other: ("Cancel", "xmark", { [weak self] in self?.model.cancelTalk() })),
            state(.sending, ["Sending"], "arrow.up.circle.fill", talk: nil),
            state(.speaking, with.map { ["\($0) is speaking", "Speaking"] } ?? ["Speaking"], "speaker.wave.2.fill", picture: model.only.flatMap { avatar(of: $0, side: 120) }, talk: ("Talk", "mic.fill"), other: ("Quiet", "speaker.slash.fill", { [weak self] in self?.model.voice.hush() })),
            state(.offline, ["Can\u{2019}t reach your computer", "Offline"], "wifi.slash", talk: nil),
        ])
        if #available(iOS 26.4, *) {
            screen.leadingNavigationBarButtons = [CPBarButton(title: "End") { [weak self] _ in
                guard let self else { return }
                Task { @MainActor in await self.model.setCalling(false) }
            }]
            // A button's words can change while the screen is up, which a state's cannot: this one says who
            // is speaking, or who he is answering.
            let who = CPBarButton(title: with ?? "") { _ in }
            who.isEnabled = false
            // and its badge beside its name, as on the phone
            let face = CPBarButton(image: model.only.flatMap { avatar(of: $0, side: 28) } ?? UIImage()) { _ in }
            face.isEnabled = false
            screen.trailingNavigationBarButtons = [face, who]
            (speaker, speakerFace) = (who, face)
        }
        voiceScreen = screen
        car.presentTemplate(screen, animated: true) { [weak self] shown, _ in
            if shown { self?.show() } else { self?.voiceScreen = nil }
        }
    }

    private func close() {
        guard voiceScreen != nil else { return }
        voiceScreen = nil
        started = false
        car?.dismissTemplate(animated: true, completion: nil)
    }

    /// One state of the voice screen: its words, its picture, and its buttons. The talk button is the same
    /// toggle as the big button on the phone: the first press opens the microphone and the second sends.
    private func state(_ id: CarState, _ titles: [String], _ symbol: String, picture: UIImage? = nil, talk: (String, String)?, other: (String, String, () -> Void)? = nil) -> CPVoiceControlState {
        let picture = picture ?? UIImage(systemName: symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 72, weight: .semibold))
        let made = CPVoiceControlState(identifier: id.rawValue, titleVariants: titles, image: picture, repeats: false)
        if #available(iOS 26.4, *) {
            var buttons: [CPButton] = []
            if let talk {
                let button = CPButton(image: UIImage(systemName: talk.1) ?? UIImage()) { [weak self] _ in
                    guard let self else { return }
                    Task { @MainActor in await self.model.talk() }
                }
                button.title = talk.0
                buttons.append(button)
            }
            if let other {
                let button = CPButton(image: UIImage(systemName: other.1) ?? UIImage()) { _ in other.2() }
                button.title = other.0
                buttons.append(button)
            }
            made.actionButtons = Array(buttons.prefix(CPVoiceControlState.maximumActionButtonCount))
        }
        return made
    }

    /// An agent's badge as a picture, found by its id: its two letters on its team's colour, as the phone shows it.
    private func avatar(of agent: String, side: CGFloat) -> UIImage? {
        guard let found = model.badge(of: agent) else { return nil }
        return badgePicture(found.0, fill: UIColor(Palette.team(found.1)), side: side).withRenderingMode(.alwaysOriginal)
    }

    /// Show the state the conversation is in. The screen may drop a change that comes too soon after another,
    /// so what it shows is looked at again a moment later.
    private func show() {
        guard let screen = voiceScreen else { return }
        let wanted: CarState
        switch model.status {
        case .offline: wanted = .offline
        case .listening: wanted = .listening
        case .sending: wanted = .sending
        case .speaking: wanted = .speaking
        case .idle: wanted = model.micOpen ? .listening : .idle
        }
        if case .speaking(let agent, let callsign?) = model.status, speaker?.title != callsign {
            speaker?.title = callsign
            if let agent, let face = avatar(of: agent, side: 28) { speakerFace?.image = face }
        }
        guard screen.activeStateIdentifier != wanted.rawValue else { return }
        screen.activateVoiceControlState(withIdentifier: wanted.rawValue)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) { [weak self] in
            guard let self, self.voiceScreen === screen, screen.activeStateIdentifier != wanted.rawValue else { return }
            self.show()
        }
    }
}

/// The states the car's voice screen has; the screen knows each by its identifier.
private enum CarState: String {
    case idle, listening, sending, speaking, offline
}
