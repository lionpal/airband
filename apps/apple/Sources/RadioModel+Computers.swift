import Foundation

/// His computers as the screens show them: the picture of each, and which is his main one.
extension RadioModel {
    /// The pictures a computer can have, in the order the picker shows them.
    static let computerPictures: [ComputerPicture] = [.laptop, .mini, .studio, .imac, .pro, .plain, .pc, .server, .chip]

    /// The system symbol for a picture, as the core gives it.
    static func symbol(_ picture: ComputerPicture) -> String {
        switch picture {
        case .laptop: "laptopcomputer"
        case .mini: "macmini"
        case .studio: "macstudio"
        case .imac: "desktopcomputer"
        case .pro: "macpro.gen3"
        case .plain: "display"
        case .pc: "pc"
        case .server: "server.rack"
        case .chip: "cpu"
        }
    }
    /// The symbol for a computer, by its id.
    func icon(of computer: String) -> String { Self.symbol(view.stations.first { $0.id == computer }?.picture ?? .plain) }
    /// What the default computer is for, in the words used wherever he chooses it.
    static let mainComputerNote = "New agents start here."
    func setMainComputer(_ computer: String) {
        guard !computer.isEmpty else { return }
        desk.setMainComputer(computer: computer)
    }
    func setPicture(_ picture: ComputerPicture, of computer: String) {
        desk.setComputerPicture(computer: computer, picture: picture)
    }
}
