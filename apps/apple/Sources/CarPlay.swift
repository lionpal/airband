import CarPlay
import SwiftUI
import Combine
import UIKit

/// The car's screen, drawn from the core's [`CarView`] and nothing else: what is listed, in what order, what
/// each row says and what a tap does are the core's. This file only turns it into Apple's templates and sends
/// a tap back as the act the core gave.
///
/// Apple's rules for an app whose main way in is the voice shape it: the voice screen comes up first, no more
/// than three screens are up at once (the core's `carFits` says, and every push and every screen put over
/// another asks it first), and only the templates Apple lists for this kind of app are used ([`CarTemplates`]).
/// The microphone is used only while the voice screen is up, and the audio is held only for as long as the
/// conversation is on. Nothing here asks him to pick up his phone, or to type.
final class CarPlaySceneDelegate: UIResponder, CPTemplateApplicationSceneDelegate, CPInterfaceControllerDelegate {
    private var car: CPInterfaceController?
    private var tabBar: CPTabBarTemplate?
    /// The agent whose screen is pushed, by id, and that screen.
    private var agentScreen: (id: String, list: CPListTemplate)?
    private var voiceScreen: CPVoiceControlTemplate?
    /// The button on the voice screen that names who is speaking, and the one with its badge.
    private var speaker: CPBarButton?
    private var speakerFace: CPBarButton?
    /// What is up, first to last, as the core counts screens.
    private var stack = CarStack()
    private var shown: CarView?
    private var watching: Set<AnyCancellable> = []
    private var model: RadioModel { RadioModel.shared }
    /// The conversation was started from the car: it ends when the car goes.
    private var started = false
    /// The screens a development build was launched to show (`-airband.devCar "agent shop two"`), until they can be.
    private var devShot: String?
    /// And the voice screen over the agent's, once that is up.
    private var devVoice = false

    func templateApplicationScene(_ scene: CPTemplateApplicationScene, didConnect controller: CPInterfaceController) {
        car = controller
        controller.delegate = self
        model.voice.inCar = true
        let view = carView()
        let tabs = CarTemplates.tabs(view, act: { [weak self] in self?.act($0) })
        tabBar = tabs
        shown = view
        stack = CarStack()
        controller.setRootTemplate(tabs, animated: false, completion: nil)
        model.$view.receive(on: DispatchQueue.main).sink { [weak self] _ in self?.redraw() }.store(in: &watching)
        model.$status.combineLatest(model.$micOpen).receive(on: DispatchQueue.main).sink { [weak self] _ in self?.show() }.store(in: &watching)
        model.$calling.removeDuplicates().receive(on: DispatchQueue.main).sink { [weak self] on in if !on { self?.close() } }.store(in: &watching)
        // a conversation already on when the car connects carries on, on the car's screen; otherwise it opens
        // as the core says, on the voice screen
        if let shot = Dev.text("devCar") {
            // a development build launched for a screenshot: these screens, without a tap
            devShot = shot
            redraw()
        } else if model.calling { open() } else if let launch = view.launch { act(launch) }
    }

    func templateApplicationScene(_ scene: CPTemplateApplicationScene, didDisconnectInterfaceController controller: CPInterfaceController) {
        watching.removeAll()
        if started && model.calling { Task { @MainActor in await self.model.setCalling(false) } }
        started = false
        model.voice.inCar = false
        (car, tabBar, agentScreen, voiceScreen, shown) = (nil, nil, nil, nil, nil)
        stack = CarStack()
    }

    /// He went back from an agent's screen.
    func templateDidDisappear(_ template: CPTemplate, animated: Bool) {
        // gone from the navigation, not only covered by the voice screen
        guard let open = agentScreen, template === open.list, !(car?.templates.contains { $0 === open.list } ?? false) else { return }
        agentScreen = nil
        stack.gone(.agent)
    }

    /// The Talk tile on the car's dashboard, or a banner, was tapped: go straight into the conversation with
    /// the microphone open, for the agent the banner was about if there was one.
    func scene(_ scene: UIScene, openURLContexts contexts: Set<UIOpenURLContext>) {
        guard let url = contexts.first?.url, let route = OpenWith.route(url), model.view.paired else { return }
        if case .answer(let agent?) = route {
            // its question is read out, then the microphone opens for the answer
            converse(agent: agent) { self.model.answer(agent) }
        } else {
            // the Talk tile, or a banner about no one in particular: everyone, and no message picked
            act(.talk(agent: nil, re: nil, listen: true))
        }
    }

    // MARK: the core's view

    /// The core's view, with the screen of the agent that is open, or of this one.
    private func carView(_ agent: String? = nil) -> CarView {
        model.desk.car(agent: agent ?? agentScreen?.id, room: CarTemplates.room)
    }

    /// Draw again what changed: the tabs, and the agent's screen (gone back from, once that agent has gone).
    private func redraw() {
        guard let tabBar else { return }
        let view = carView()
        if let shot = devShot { dev(shot, view) }
        guard view != shown else { return }
        let before = shown
        shown = view
        if view.tabs.map(\.kind) == before?.tabs.map(\.kind) {
            for (template, tab) in zip(tabBar.templates, view.tabs) {
                (template as? CPListTemplate)?.updateSections(CarTemplates.sections(tab.sections, act: { [weak self] in self?.act($0) }))
                template.showsTabBadge = tab.needsYou
            }
        } else {
            tabBar.updateTemplates(CarTemplates.tabs(view, act: { [weak self] in self?.act($0) }).templates)
        }
        guard let open = agentScreen else { return }
        if let agent = view.agent {
            open.list.updateSections(CarTemplates.sections(agent.sections, act: { [weak self] in self?.act($0) }))
        } else if voiceScreen == nil {
            agentScreen = nil
            stack.gone(.agent)
            car?.popToRootTemplate(animated: true, completion: nil)
        }
    }

    /// For a screenshot: a tab ("teams"), or an agent's screen by what it is called ("agent shop two"), with
    /// the voice screen over it ("voice shop two"). Waits for the agent to be on the list.
    private func dev(_ shot: String, _ view: CarView) {
        let words = shot.split(separator: " ", maxSplits: 1).map(String.init)
        let kinds: [String: CarTabKind] = ["agents": .agents, "teams": .teams, "computers": .computers]
        if let kind = kinds[words[0]], let tabBar, let at = view.tabs.firstIndex(where: { $0.kind == kind }), at < tabBar.templates.count {
            devShot = nil
            tabBar.select(tabBar.templates[at])
            return
        }
        guard words.count == 2 else { return }
        let rows = view.tabs.flatMap { $0.sections.flatMap(\.rows) }
        guard case .open(let agent)? = rows.first(where: { $0.text == words[1] })?.act else { return }
        devShot = nil
        devVoice = words[0] == "voice"
        DispatchQueue.main.async { self.act(.open(agent: agent)) }
    }

    /// Do what a tap stood for, as the core gave it.
    private func act(_ act: CarAct) {
        switch act {
        case .open(let agent):
            let view = carView(agent)
            guard let car, stack.fits(.agent), let screen = view.agent else { return }
            let list = CarTemplates.agent(screen, act: { [weak self] in self?.act($0) })
            agentScreen = (agent, list)
            shown = view
            stack.push(.agent)
            car.pushTemplate(list, animated: true) { [weak self] pushed, _ in
                guard let self else { return }
                if !pushed {
                    self.agentScreen = nil
                    self.stack.gone(.agent)
                }
                if pushed && self.devVoice {
                    self.devVoice = false
                    self.act(.talk(agent: agent, re: nil, listen: false))
                }
            }
        case .talk(let agent, let re, let listen):
            converse(agent: agent) {
                if listen && !self.model.micOpen { Task { @MainActor in await self.model.talk(re: re) } }
            }
        case .read(let agent, let message):
            converse(agent: agent) { self.model.readAloud(message) }
        case .tap(let message, let tapped):
            model.desk.tap(messageId: message, tapped: tapped)
        }
    }

    // MARK: the conversation

    /// Look at the agent (or everyone), turn the conversation on, put up the voice screen, then `then`.
    private func converse(agent: String?, then: @escaping () -> Void) {
        Task { @MainActor in
            if let agent { self.model.look(agent: agent) } else { self.model.look() }
            // in the car he is told on the car's screen, not the phone's
            if !self.model.calling { await self.model.setCalling(true, tell: false) }
            guard self.model.calling else {
                // the microphone is not allowed. It can only be allowed on the phone, which he is not asked to touch
                guard self.stack.fits(.alert) else { return }
                self.stack.push(.alert)
                let ok = CPAlertAction(title: "OK", style: .cancel) { [weak self] _ in
                    self?.stack.gone(.alert)
                    self?.car?.dismissTemplate(animated: true, completion: nil)
                }
                self.car?.presentTemplate(CPAlertTemplate(titleVariants: ["airband can\u{2019}t use the microphone", "No microphone"], actions: [ok]), animated: true, completion: nil)
                return
            }
            self.started = true
            self.open()
            then()
        }
    }

    /// Put up the voice screen for the conversation that is on.
    private func open() {
        guard voiceScreen == nil, let car, stack.fits(.voice) else { return }
        let voice = carView().voice
        let screen = CarTemplates.voice(voice, badge: { [weak self] side in voice.agent.flatMap { self?.badge(of: $0, side: side) } }, press: { [weak self] in self?.press($0) })
        if #available(iOS 26.4, *) {
            screen.leadingNavigationBarButtons = [CPBarButton(title: voice.end) { [weak self] _ in
                guard let self else { return }
                Task { @MainActor in await self.model.setCalling(false) }
            }]
            // A button's words can change while the screen is up, which a state's cannot: this one says who
            // is speaking, or who he is talking with.
            let who = CPBarButton(title: voice.with) { _ in }
            who.isEnabled = false
            // and its badge beside its name, as on the phone
            let face = CPBarButton(image: voice.agent.flatMap { badge(of: $0, side: 28) } ?? UIImage()) { _ in }
            face.isEnabled = false
            screen.trailingNavigationBarButtons = [face, who]
            (speaker, speakerFace) = (who, face)
        }
        voiceScreen = screen
        stack.push(.voice)
        car.presentTemplate(screen, animated: true) { [weak self] up, _ in
            if up { self?.show() } else { self?.voiceScreen = nil; self?.stack.gone(.voice) }
        }
    }

    private func close() {
        guard voiceScreen != nil else { return }
        voiceScreen = nil
        stack.gone(.voice)
        started = false
        car?.dismissTemplate(animated: true, completion: nil)
        redraw()
    }

    /// A button on the voice screen.
    private func press(_ action: CarVoiceAction) {
        switch action {
        case .talk: Task { @MainActor in await self.model.talk() }
        case .cancel: model.cancelTalk()
        case .quiet: model.voice.hush()
        }
    }

    /// An agent's badge as a picture, found by its id: its two letters on its team's colour, as the phone shows it.
    private func badge(of agent: String, side: CGFloat) -> UIImage? {
        model.badge(of: agent).map { CarTemplates.badge($0.0, colour: $0.1, side: side) }
    }

    /// Show the state the conversation is in, as the core says. The screen may drop a change that comes too
    /// soon after another, so what it shows is looked at again a moment later.
    private func show() {
        guard let screen = voiceScreen else { return }
        let wanted = CarTemplates.identifier(carVoiceKind(status: model.status, micOpen: model.micOpen))
        if case .speaking(let agent, let callsign) = model.status, speaker?.title != (callsign ?? "") {
            // one speaking with no callsign (the desk itself) shows no name: never the last one's
            speaker?.title = callsign ?? ""
            speakerFace?.image = agent.flatMap { badge(of: $0, side: 28) } ?? UIImage()
        }
        guard screen.activeStateIdentifier != wanted else { return }
        screen.activateVoiceControlState(withIdentifier: wanted)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) { [weak self] in
            guard let self, self.voiceScreen === screen, screen.activeStateIdentifier != wanted else { return }
            self.show()
        }
    }
}

/// What is up on the car, first to last, as the core counts screens: nothing goes up that the core's
/// `carFits` refuses.
struct CarStack {
    private(set) var screens: [CarScreen] = [.tabs]
    func fits(_ next: CarScreen) -> Bool { carFits(screens: screens + [next]) }
    mutating func push(_ next: CarScreen) { screens.append(next) }
    /// That screen has gone, and whatever was over it.
    mutating func gone(_ screen: CarScreen) {
        if screen != .tabs, let at = screens.lastIndex(of: screen) { screens.removeSubrange(at...) }
    }
}

/// The core's car view as Apple's templates. Only the templates Apple lists for an app whose main way in is
/// the voice are made here.
enum CarTemplates {
    /// What Apple's CarPlay Developer Guide lists for this kind of app (driving task and voice-based
    /// conversational): action sheet, alert, grid, list, tab bar, voice control (iOS 26.4 on), information and
    /// point of interest. Any other throws when it is shown.
    static let allowed: [CPTemplate.Type] = [CPActionSheetTemplate.self, CPAlertTemplate.self, CPGridTemplate.self, CPListTemplate.self, CPTabBarTemplate.self, CPVoiceControlTemplate.self, CPInformationTemplate.self, CPPointOfInterestTemplate.self]

    /// How much this car can show.
    static var room: CarRoom {
        CarRoom(rows: UInt32(max(0, CPListTemplate.maximumItemCount)), sections: UInt32(max(0, CPListTemplate.maximumSectionCount)), tabs: UInt32(max(0, CPTabBarTemplate.maximumTabCount)))
    }

    static func tabs(_ view: CarView, act: @escaping (CarAct) -> Void) -> CPTabBarTemplate {
        CPTabBarTemplate(templates: view.tabs.map { tab in
            let list = CPListTemplate(title: tab.title, sections: sections(tab.sections, act: act))
            list.tabTitle = tab.title
            list.tabImage = UIImage(systemName: tab.symbol)
            list.showsTabBadge = tab.needsYou
            return list
        })
    }

    static func agent(_ agent: CarAgent, act: @escaping (CarAct) -> Void) -> CPListTemplate {
        CPListTemplate(title: agent.title, sections: sections(agent.sections, act: act))
    }

    static func sections(_ sections: [CarSection], act: @escaping (CarAct) -> Void) -> [CPListSection] {
        sections.map { section in
            let items = section.rows.map { row -> CPListItem in
                let item = CPListItem(text: row.text, detailText: row.detail, image: row.picture.flatMap(picture))
                if let tap = row.act {
                    item.handler = { _, done in
                        act(tap)
                        done()
                    }
                }
                return item
            }
            return CPListSection(items: items, header: section.header, sectionIndexTitle: nil)
        }
    }

    static func picture(_ picture: CarPicture) -> UIImage? {
        switch picture {
        case .badge(let text, let colour): badge(text, colour: colour, side: 44)
        case .computer(let shape): UIImage(systemName: RadioModel.symbol(shape))
        case .symbol(let name): UIImage(systemName: name)
        }
    }

    static func badge(_ text: String, colour: UInt8?, side: CGFloat) -> UIImage {
        badgePicture(text, fill: UIColor(Palette.team(colour)), side: side).withRenderingMode(.alwaysOriginal)
    }

    /// The voice screen, each state with the core's words, mark and buttons. The talk button is the same
    /// toggle as the big button on the phone: the first press opens the microphone and the second sends.
    static func voice(_ voice: CarVoice, badge: @escaping (CGFloat) -> UIImage?, press: @escaping (CarVoiceAction) -> Void) -> CPVoiceControlTemplate {
        CPVoiceControlTemplate(voiceControlStates: voice.states.map { state in
            let picture = (state.badge ? badge(120) : nil) ?? UIImage(systemName: state.symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 72, weight: .semibold))
            let made = CPVoiceControlState(identifier: identifier(state.kind), titleVariants: state.titles, image: picture, repeats: false)
            if #available(iOS 26.4, *) {
                made.actionButtons = state.buttons.prefix(CPVoiceControlState.maximumActionButtonCount).map { button in
                    let b = CPButton(image: UIImage(systemName: button.symbol) ?? UIImage()) { _ in press(button.action) }
                    b.title = button.title
                    return b
                }
            }
            return made
        })
    }

    /// The identifier the voice screen knows a state by.
    static func identifier(_ kind: CarVoiceKind) -> String {
        switch kind {
        case .ready: "ready"
        case .listening: "listening"
        case .sending: "sending"
        case .speaking: "speaking"
        case .offline: "offline"
        }
    }
}
