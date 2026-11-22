import SwiftUI

struct SetupView: View {
    @EnvironmentObject var model: RadioModel
    @ObservedObject private var push = Push.shared
    @Environment(\.dismiss) private var dismiss
    // development only: `-airband.devAccounts 1` opens the accounts page, for a screenshot
    @State private var showAccounts = UserDefaults.standard.bool(forKey: "airband.devAccounts")
    // development only: `-airband.devPage Voice` opens Settings on that page (or `Settings` for the list itself)
    @State private var devPage = UserDefaults.standard.string(forKey: "airband.devPage")
    @State private var scanning = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    // one row for each job the core lists, opening the same picker the first run showed
                    ForEach(model.jobs, id: \.self) { job in
                        NavigationLink(isActive: Binding(get: { devPage == job.title }, set: { if !$0 { devPage = nil } })) {
                            ChoicePage(title: job.title, headline: job.headline) { ChoicePicker(job: job, mode: .settings) }.environmentObject(model)
                        } label: {
                            row(job.title, model.chosenLabel(job))
                        }
                    }
                }
                Section {
                    NavigationLink {
                        ChoicePage(title: "Coding agents", headline: Headline.tools) {
                            VStack(alignment: .leading, spacing: 14) {
                                Text("Tick the ones you use. What can write your summaries follows from them.").font(.system(size: 15)).foregroundStyle(Palette.soft)
                                ToolsPicker(single: false)
                                ForEach(model.usedTools.filter { !$0.first.isEmpty }, id: \.id) { tool in
                                    Text(tool.first).font(.system(size: 13)).foregroundStyle(Palette.quiet)
                                }
                            }
                        }
                        .environmentObject(model)
                    } label: {
                        row("Coding agents", model.usedTools.map(\.name).joined(separator: ", "))
                    }
                }
                Section {
                    Toggle("Show whole messages", isOn: $model.showWhole)
                } footer: {
                    Text("Off, you see the short version of what each agent wrote and can open any one. On, you see all of it. What is read aloud is the short version either way.")
                }
                if !model.computers.isEmpty {
                    Section {
                        if model.computers.count > 1 {
                            Picker("Main computer", selection: Binding(get: { model.view.mainComputer ?? "" }, set: { model.setMainComputer($0) })) {
                                if model.view.mainComputer == nil { Text("Not chosen").tag("") }
                                ForEach(model.computers, id: \.self) { name in Text(name).tag(name) }
                            }
                        }
                        ForEach(model.computers, id: \.self) { name in
                            Picker(selection: Binding(get: { model.icon(of: name) }, set: { model.setIcon($0, of: name) })) {
                                ForEach(RadioModel.computerSymbols, id: \.self) { symbol in Image(systemName: symbol).tag(symbol) }
                            } label: {
                                Label(name, systemImage: model.icon(of: name))
                            }
                        }
                    } header: {
                        Text("Computers")
                    } footer: {
                        if model.computers.count > 1 { Text(RadioModel.mainComputerNote) }
                    }
                }
                if model.paired {
                    Section {
                        NavigationLink("Connection") { ConnectionPage().environmentObject(model) }
                        NavigationLink("Accounts", isActive: $showAccounts) { AccountsView().environmentObject(model) }
                        if push.denied {
                            Button("Turn on notifications") { UIApplication.shared.open(URL(string: UIApplication.openNotificationSettingsURLString)!) }
                        }
                    }
                }
                Section(footer: Text("Run ") + Text("airband pair").font(.system(.footnote, design: .monospaced)) + Text(" on each computer, then scan the code it shows or paste its link. Each one adds that computer. Treat a link like a password.")) {
                    Button { scanning = true } label: { Label("Scan code", systemImage: "qrcode.viewfinder").font(.body.weight(.semibold)) }
                    // the other way in, for a link sent to the phone: smaller, under the scan
                    Button("Paste a link instead") {
                        model.pair(with: UIPasteboard.general.string ?? "")
                        if model.problem == nil { dismiss() }
                    }
                    .font(.footnote).foregroundStyle(.secondary)
                    if model.paired {
                        Text(model.computers.isEmpty ? "Paired" : "Paired: " + model.computers.joined(separator: ", ")).foregroundStyle(.secondary)
                        Button("Forget all", role: .destructive) { model.forgetAll() }
                    }
                    if let problem = model.problem { Text(problem).foregroundStyle(.red) }
                }
            }
            .fullScreenCover(isPresented: $scanning) { ScanSheet().environmentObject(model) }
            .navigationTitle("Settings").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
    }
}
extension SetupView {
    /// One row of the settings list: what it is, and what is chosen for it now.
    func row(_ title: String, _ now: String) -> some View {
        HStack { Text(title); Spacer(); Text(now).foregroundStyle(.secondary).lineLimit(1) }
    }
}
/// How each computer was last reached, and the record to copy when something is wrong. All of it is what this
/// phone saw itself and kept. Nothing here is sent anywhere.
struct ConnectionPage: View {
    @EnvironmentObject var model: RadioModel
    @State private var reach: [DeskReach] = []
    @State private var copied = false

    var body: some View {
        Form {
            ForEach(reach, id: \.computer) { r in
                Section(r.computer) {
                    line("Through the relay", r.relay)
                    line("On this network", r.near)
                }
            }
            Section {
                Button(copied ? "Copied" : "Copy the connection record") {
                    UIPasteboard.general.string = model.diagnostics()
                    copied = true
                }
            } footer: {
                Text("The record lists requests that failed and each try at sending what you said, with the time, the way it went and how it ended. Nothing you or your agents said is in it. It stays on this phone unless you paste it somewhere.")
            }
        }
        .navigationTitle("Connection").navigationBarTitleDisplayMode(.inline)
        .onAppear { reach = model.reach() }
    }

    private func line(_ title: String, _ last: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
            Text(last.isEmpty ? "Nothing has failed" : last.prefix(1).uppercased() + last.dropFirst()).font(.footnote).foregroundStyle(.secondary)
        }
    }
}
/// The accounts his agents run under, on each computer: how much of each is used, and a tap to move every
/// session on that computer to another one. Signing in to a new account is done at the computer.
struct AccountsView: View {
    @EnvironmentObject var model: RadioModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                ForEach(model.accounts, id: \.station) { station in
                    VStack(alignment: .leading, spacing: 10) {
                        if model.accounts.count > 1, !station.station.isEmpty {
                            Text(station.station).font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.quiet).padding(.horizontal, 6)
                        }
                        ForEach(station.accounts, id: \.id) { a in
                            Button { model.useAccount(station.station, a) } label: { row(station.station, a) }
                                .buttonStyle(.plain).allowsHitTesting(!a.live && model.switching == nil) // not disabled: that would fade the one in use
                                .accessibilityLabel(a.live ? "\(a.label), in use" : "Switch to \(a.label)")
                        }
                        if let note = station.note {
                            Text(note).font(.system(size: 14)).foregroundStyle(Palette.quiet).padding(.horizontal, 6)
                        }
                    }
                }
                if let problem = model.accountsProblem {
                    Text(problem).font(.system(size: 15)).foregroundStyle(Color.red).padding(.horizontal, 6)
                }
                if model.accounts.isEmpty {
                    Text(model.accountsLoading ? "Checking…" : "No accounts yet.").font(.system(size: 15)).foregroundStyle(Palette.quiet).padding(.horizontal, 6)
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

    private func row(_ station: String, _ a: AccountEntry) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Text(a.label).font(.system(size: 17, weight: .semibold)).lineLimit(1).truncationMode(.middle)
                Spacer(minLength: 8)
                if model.switching == station + "|" + a.id {
                    ProgressView()
                } else if a.live {
                    Text("in use").font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.signal)
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
