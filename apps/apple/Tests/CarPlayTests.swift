// The car's screens are made only of the templates Apple lets this kind of app use, and never more than three
// are up at once.
import CarPlay
import XCTest
@testable import Airband

final class CarPlayTests: XCTestCase {
    private func row(_ text: String, _ act: CarAct?) -> CarRow { CarRow(text: text, detail: nil, picture: .symbol(name: "mic.fill"), act: act) }
    private var sample: CarView {
        let agents = CarTab(kind: .agents, title: "Agents", symbol: "person.2.fill", needsYou: true, sections: [
            CarSection(header: nil, rows: [row("Talk to everyone", .talk(agent: nil, re: nil, listen: true))]),
            CarSection(header: "Waiting on you", rows: [CarRow(text: "shop two", detail: nil, picture: .badge(badge: "S2", colour: 1), act: .open(agent: "a2"))]),
        ])
        let teams = CarTab(kind: .teams, title: "Teams", symbol: "square.grid.2x2.fill", needsYou: true, sections: [
            CarSection(header: "shop", rows: [CarRow(text: "shop two", detail: "Waiting on you", picture: .badge(badge: "S2", colour: 1), act: .open(agent: "a2"))]),
        ])
        let computers = CarTab(kind: .computers, title: "Computers", symbol: "desktopcomputer", needsYou: false, sections: [
            CarSection(header: nil, rows: [CarRow(text: "studio", detail: "Can\u{2019}t reach", picture: .computer(picture: .studio), act: nil)]),
        ])
        let agent = CarAgent(id: "a2", title: "shop two", sections: [
            CarSection(header: "Answer", rows: [row("Allow", .tap(message: "q", tapped: .allow)), row("Deny", .tap(message: "q", tapped: .deny))]),
            CarSection(header: nil, rows: [row("Reply", .talk(agent: "a2", re: "q", listen: true)), row("Read aloud", .read(agent: "a2", message: "q"))]),
            CarSection(header: "Latest", rows: [row("It wants to push", .read(agent: "a2", message: "q"))]),
        ])
        let states = [CarVoiceKind.ready, .listening, .sending, .speaking, .offline].map { CarVoiceState(kind: $0, titles: ["Waiting"], symbol: "mic.fill", badge: false, buttons: [CarVoiceButton(action: .talk, title: "Talk", symbol: "mic.fill")]) }
        return CarView(paired: true, launch: .talk(agent: nil, re: nil, listen: false), tabs: [agents, teams, computers], agent: agent, voice: CarVoice(agent: "a2", with: "shop two", states: states, end: "End"))
    }

    private func allowed(_ template: CPTemplate) -> Bool {
        CarTemplates.allowed.contains { type(of: template) == $0 }
    }

    func testEveryScreenIsATemplateAppleAllowsThisKindOfApp() {
        let view = sample
        let tabs = CarTemplates.tabs(view, act: { _ in })
        let agent = CarTemplates.agent(view.agent!, act: { _ in })
        let voice = CarTemplates.voice(view.voice, badge: { _ in nil }, press: { _ in })
        for template in [tabs, agent, voice] + tabs.templates { XCTAssertTrue(allowed(template), "\(type(of: template))") }
        XCTAssertEqual(tabs.templates.count, 3)
        XCTAssertEqual(voice.voiceControlStates.map(\.identifier), ["ready", "listening", "sending", "speaking", "offline"])
        // Apple's list, and nothing it leaves out for this kind of app
        for refused: CPTemplate.Type in [CPNowPlayingTemplate.self, CPContactTemplate.self, CPMapTemplate.self, CPSearchTemplate.self] {
            XCTAssertFalse(CarTemplates.allowed.contains { $0 == refused }, "\(refused)")
        }
    }

    func testNoTapEverPutsUpMoreThanThreeScreens() {
        let view = sample
        let opens: [(CarStack, CarAct)] = {
            var onTabs: [(CarStack, CarAct)] = view.tabs.flatMap { $0.sections.flatMap(\.rows) }.compactMap(\.act).map { (CarStack(), $0) }
            onTabs += view.launch.map { [(CarStack(), $0)] } ?? []
            var agent = CarStack()
            agent.push(.agent)
            return onTabs + view.agent!.sections.flatMap(\.rows).compactMap(\.act).map { (agent, $0) }
        }()
        for (stack, act) in opens {
            var stack = stack
            if let next = carOpens(act: act) {
                XCTAssertTrue(stack.fits(next), "\(act)")
                stack.push(next)
            }
            XCTAssertLessThanOrEqual(stack.screens.count, 3)
        }
        // what would go past three, or out of order, is refused
        var full = CarStack()
        full.push(.agent)
        full.push(.voice)
        XCTAssertFalse(full.fits(.voice))
        XCTAssertFalse(full.fits(.alert))
        XCTAssertFalse(full.fits(.agent))
        var tabs = CarStack()
        tabs.push(.voice)
        XCTAssertFalse(tabs.fits(.agent))
    }
}
