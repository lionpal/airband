// The banners on the lock screen, and on a car's screen, for agents waiting on him during a call; and the tile.
import ActivityKit
import Foundation
import WidgetKit

extension RadioModel {
    /// While he is on a call, each agent that is waiting on him has a banner: on the lock screen, and on a
    /// car's screen when another app is in front. It comes down when the agent has its answer or the call ends.
    func syncBanners() {
        let waiting = calling ? agents.filter { $0.state == .asking } : []
        for (id, banner) in banners where !waiting.contains(where: { $0.id == id }) {
            banners[id] = nil
            Task { await banner.end(nil, dismissalPolicy: .immediate) }
        }
        guard !waiting.isEmpty, ActivityAuthorizationInfo().areActivitiesEnabled else { return }
        // the ones already shown say how many wait now and what each asks now, quietly
        for agent in waiting {
            let state = NeedsYouAttributes.ContentState(waiting: waiting.count, question: agent.question)
            guard let banner = banners[agent.id], banner.content.state != state else { continue }
            Task { await banner.update(.init(state: state, staleDate: nil)) }
        }
        for agent in waiting where banners[agent.id] == nil {
            let state = NeedsYouAttributes.ContentState(waiting: waiting.count, question: agent.question)
            guard let banner = try? Activity.request(attributes: NeedsYouAttributes(agent: agent.id, callsign: agent.callsign, badge: agent.badge), content: .init(state: state, staleDate: nil)) else { continue }
            banners[agent.id] = banner
            // the alert is what makes it show itself, and not only sit on the lock screen
            Task { await banner.update(.init(state: state, staleDate: nil), alertConfiguration: .init(title: "\(agent.callsign) needs you", body: LocalizedStringResource(stringLiteral: agent.question.isEmpty ? "Tap to answer" : agent.question), sound: .default)) }
        }
    }

    /// The tile holds what the chat now says (the core decides what that is): kept in the shared folder when it
    /// changes, and drawn again when what it shows does.
    func syncTile() {
        let facts = desk.tile()
        guard facts != keptTile, let folder = Together.folder else { return }
        keptTile = facts
        let redraw = folder.keepTile(facts: facts)
        Board.protect(folder.tilePath())
        if redraw { Board.redraw() }
    }
}
