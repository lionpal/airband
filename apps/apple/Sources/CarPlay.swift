import CarPlay
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
    private var watching: Set<AnyCancellable> = []
    private var model: RadioModel { RadioModel.shared }
    /// The conversation was started from the car: it ends when the car goes.
    private var started = false

    func templateApplicationScene(_ scene: CPTemplateApplicationScene, didConnect controller: CPInterfaceController) {
        car = controller
        model.voice.inCar = true
        controller.setRootTemplate(root(), animated: false, completion: nil)
        // what the list says follows whether he is paired; what the voice screen says follows the conversation
        model.$view.map(\.paired).removeDuplicates().receive(on: DispatchQueue.main).sink { [weak self] _ in self?.refreshRoot() }.store(in: &watching)
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

    // MARK: the list

    private func root() -> CPListTemplate {
        let item: CPListItem
        if model.view.paired {
            item = CPListItem(text: "Talk to your agents", detailText: nil, image: UIImage(systemName: "mic.fill"))
            item.handler = { [weak self] _, done in
                self?.start()
                done()
            }
        } else {
            item = CPListItem(text: "Not paired with a computer yet", detailText: nil)
            item.isEnabled = false
        }
        let list = CPListTemplate(title: "airband", sections: [CPListSection(items: [item])])
        return list
    }

    private func refreshRoot() {
        guard voiceScreen == nil else { return }
        car?.setRootTemplate(root(), animated: false, completion: nil)
    }

    // MARK: the conversation

    private func start() {
        Task { @MainActor in
            await self.model.setCalling(true)
            guard self.model.calling else {
                // the microphone is not allowed. It can only be allowed on the phone, which he is not asked to touch
                let ok = CPAlertAction(title: "OK", style: .cancel) { [weak self] _ in self?.car?.dismissTemplate(animated: true, completion: nil) }
                self.car?.presentTemplate(CPAlertTemplate(titleVariants: ["airband is not allowed to use the microphone", "Microphone not allowed"], actions: [ok]), animated: true, completion: nil)
                self.model.problem = nil
                return
            }
            self.started = true
            self.open()
        }
    }

    /// Put up the voice screen for the conversation that is on.
    private func open() {
        guard voiceScreen == nil, let car else { return }
        let screen = CPVoiceControlTemplate(voiceControlStates: [
            state("idle", ["Waiting for your agents", "Waiting"], "ellipsis", talk: "Talk"),
            state("listening", ["Listening"], "mic.fill", talk: "Send", other: ("Cancel", { [weak self] in self?.model.cancelTalk() })),
            state("sending", ["Sending"], "arrow.up.circle.fill", talk: nil),
            state("speaking", ["Speaking"], "speaker.wave.2.fill", talk: "Talk", other: ("Quiet", { [weak self] in self?.model.voice.hush() })),
            state("offline", ["Can\u{2019}t reach your computer", "Offline"], "wifi.slash", talk: nil),
        ])
        if #available(iOS 26.4, *) {
            screen.leadingNavigationBarButtons = [CPBarButton(title: "End") { [weak self] _ in
                guard let self else { return }
                Task { @MainActor in await self.model.setCalling(false) }
            }]
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
    private func state(_ id: String, _ titles: [String], _ symbol: String, talk: String?, other: (String, () -> Void)? = nil) -> CPVoiceControlState {
        let picture = UIImage(systemName: symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 72, weight: .semibold))
        let made = CPVoiceControlState(identifier: id, titleVariants: titles, image: picture, repeats: false)
        if #available(iOS 26.4, *) {
            var buttons: [CPButton] = []
            if let talk {
                let button = CPButton(image: UIImage(systemName: talk == "Send" ? "arrow.up" : "mic.fill") ?? UIImage()) { [weak self] _ in
                    guard let self else { return }
                    Task { @MainActor in await self.model.talk() }
                }
                button.title = talk
                buttons.append(button)
            }
            if let other {
                let button = CPButton(image: UIImage(systemName: other.0 == "Cancel" ? "xmark" : "speaker.slash.fill") ?? UIImage()) { _ in other.1() }
                button.title = other.0
                buttons.append(button)
            }
            made.actionButtons = Array(buttons.prefix(CPVoiceControlState.maximumActionButtonCount))
        }
        return made
    }

    /// Show the state the conversation is in. The screen may drop a change that comes too soon after another,
    /// so what it shows is looked at again a moment later.
    private func show() {
        guard let screen = voiceScreen else { return }
        let wanted: String
        switch model.status {
        case .offline: wanted = "offline"
        case .listening: wanted = "listening"
        case .sending: wanted = "sending"
        case .speaking: wanted = "speaking"
        case .idle: wanted = model.micOpen ? "listening" : "idle"
        }
        guard screen.activeStateIdentifier != wanted else { return }
        screen.activateVoiceControlState(withIdentifier: wanted)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) { [weak self] in
            guard let self, self.voiceScreen === screen, screen.activeStateIdentifier != wanted else { return }
            self.show()
        }
    }
}
