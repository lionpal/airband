import Foundation

/// His computers as the screens show them: the shape of each, and which is his main one.
extension RadioModel {
    /// The system symbol for a computer's shape, as the core gives it. One it does not know is a plain display.
    static func symbol(_ shape: MachineShape) -> String {
        switch shape {
        case .laptop: "laptopcomputer"
        case .mini: "macmini"
        case .studio: "macstudio"
        case .imac: "desktopcomputer"
        case .pro: "macpro.gen3"
        case .unknown: "display"
        }
    }
    /// The symbol for a computer, by its id.
    func icon(of computer: String) -> String { Self.symbol(view.stations.first { $0.id == computer }?.shape ?? .unknown) }
    /// What the default computer is for, in the words used wherever he chooses it.
    static let mainComputerNote = "New agents start here."
    func setMainComputer(_ computer: String) {
        guard !computer.isEmpty else { return }
        desk.setMainComputer(computer: computer)
    }
}
