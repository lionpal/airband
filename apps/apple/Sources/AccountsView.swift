import SwiftUI

/// The accounts his agents run under, by coding tool: how much of each is used, and a tap to move every
/// session on that computer to another one. Signing in to a new account is done at the computer.
struct AccountsView: View {
    @EnvironmentObject var model: RadioModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                ForEach(accountsByTool(accounts: model.accounts), id: \.harness) { tool in
                    VStack(alignment: .leading, spacing: 10) {
                        Text(tool.name).font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.quiet).padding(.horizontal, 6)
                        ForEach(tool.rows, id: \.key) { r in
                            Button { model.useAccount(r.station, r.entry) } label: { row(r) }
                                .buttonStyle(.plain).allowsHitTesting(!r.entry.live && model.switching == nil) // not disabled: that would fade the one in use
                                .accessibilityLabel(r.entry.live ? "\(r.shown), in use" : "Switch to \(r.shown)")
                        }
                    }
                }
                ForEach(model.accounts.filter { $0.note != nil }, id: \.station) { station in
                    Text(model.view.severalComputers ? "\(station.name): \(station.note ?? "")" : station.note ?? "").font(.system(size: 14)).foregroundStyle(Palette.quiet).padding(.horizontal, 6)
                }
                if let problem = model.accountsProblem {
                    Text(problem).font(.system(size: 15)).foregroundStyle(Color.red).padding(.horizontal, 6)
                }
                if model.accounts.isEmpty {
                    Text(model.accountsLoading ? "Checking…" : "No accounts yet.").font(.system(size: 15)).foregroundStyle(Palette.quiet).padding(.horizontal, 6)
                } else if model.accounts.contains(where: { !$0.accounts.isEmpty }) {
                    Text("To add an account, sign in on the computer.").font(.system(size: 14)).foregroundStyle(Palette.quiet).padding(.horizontal, 6)
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Palette.ground.ignoresSafeArea())
        .foregroundStyle(Palette.ink)
        .navigationTitle("Accounts").navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                if model.accountsLoading { ProgressView() } else {
                    Button { model.loadAccounts(fresh: true) } label: { Image(systemName: "arrow.clockwise") }.accessibilityLabel("Check again")
                }
            }
        }
        .preferredColorScheme(.dark)
        .onAppear { model.loadAccounts(fresh: false) }
    }

    private func row(_ r: AccountRow) -> some View {
        let a = r.entry
        return VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(r.shown).font(.system(size: 17, weight: .semibold)).lineLimit(1).truncationMode(.middle)
                    if !r.computer.isEmpty {
                        Text(r.computer).font(.system(size: 13)).foregroundStyle(Palette.quiet)
                    }
                }
                Spacer(minLength: 8)
                if model.switching == RadioModel.Switching(station: r.station, account: a.id) {
                    ProgressView()
                } else if a.live {
                    Text("In use").font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.signal)
                }
            }
            if let problem = a.problem {
                Text(problem).font(.system(size: 14)).foregroundStyle(Palette.ask)
            }
            ForEach(a.limits, id: \.name) { limit in
                VStack(alignment: .leading, spacing: 5) {
                    GeometryReader { g in
                        ZStack(alignment: .leading) {
                            Capsule().fill(Palette.raised)
                            Capsule().fill(Self.colour(limit.used)).frame(width: max(6, g.size.width * limit.used))
                        }
                    }
                    .frame(height: 6)
                    HStack(spacing: 6) {
                        Text("\(limit.name) \(Int((limit.used * 100).rounded()))%")
                        if let at = limit.resetsAt {
                            Text("· resets \(Date(timeIntervalSince1970: Double(at) / 1000), format: .dateTime.weekday(.abbreviated).hour().minute())")
                        }
                    }
                    .font(.system(size: 13)).foregroundStyle(Palette.quiet)
                }
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Palette.card, in: RoundedRectangle(cornerRadius: 20))
        .overlay(RoundedRectangle(cornerRadius: 20).stroke(a.live ? Palette.signal.opacity(0.45) : .clear, lineWidth: 1))
        .contentShape(RoundedRectangle(cornerRadius: 20))
    }

    /// Green with room, amber when most of it is gone, red when nearly all.
    private static func colour(_ used: Double) -> Color {
        used >= 0.9 ? .red : used >= 0.75 ? Palette.ask : Palette.signal
    }
}

extension AccountRow {
    /// One account on one computer.
    var key: String { station + "/" + entry.harness + "/" + entry.id }
}
