import Foundation

/// His computers as the screens show them: the picture he chose for each, and which is his main one.
extension RadioModel {
    /// The pictures a computer can have.
    static let computerSymbols = ["laptopcomputer", "desktopcomputer", "display", "pc", "server.rack", "cpu"]

    func icon(of computer: String) -> String { computerIcons[computer] ?? "desktopcomputer" }
    /// What the default computer is for, in the words used wherever he chooses it.
    static let mainComputerNote = "New agents start here."
    func setMainComputer(_ computer: String) {
        guard !computer.isEmpty else { return }
        desk.setMainComputer(computer: computer)
    }
    func setIcon(_ symbol: String, of computer: String) {
        computerIcons[computer] = symbol
        Stored.keepSetting("airband.computers.icons.byid", computerIcons)
    }
}
