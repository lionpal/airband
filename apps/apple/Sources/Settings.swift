import SwiftUI

struct SetupView: View {
    @EnvironmentObject var model: RadioModel
    @ObservedObject private var push = Push.shared
    @Environment(\.dismiss) private var dismiss
    // development only: `-airband.devAccounts 1` opens the accounts page, for a screenshot
    @State private var showAccounts = Dev.flag("devAccounts")
    // development only: `-airband.devPage Voice` opens Settings on that page (or `Settings` for the list itself)
    @State private var devPage = Dev.text("devPage")
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
                    Text("What's read aloud stays short.")
                }
                if !model.computers.isEmpty {
                    Section {
                        if model.computers.count > 1 {
                            Picker("Default computer", selection: Binding(get: { model.view.mainComputer ?? "" }, set: { model.setMainComputer($0) })) {
                                if model.view.mainComputer == nil { Text("Not chosen").tag("") }
                                ForEach(model.computers, id: \.id) { c in Text(c.name).tag(c.id) }
                            }
                        }
                        ForEach(model.computers, id: \.id) { c in
                            Picker(selection: Binding(get: { model.icon(of: c.id) }, set: { model.setIcon($0, of: c.id) })) {
                                ForEach(RadioModel.computerSymbols, id: \.self) { symbol in Image(systemName: symbol).tag(symbol) }
                            } label: {
                                Text(c.name)
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
                Section(footer: Text("Run ") + Text("airband pair").font(.system(.footnote, design: .monospaced)) + Text(" on a computer, then scan its code. Keep the link private.")) {
                    Button { scanning = true } label: { Label("Scan code", systemImage: "qrcode.viewfinder").font(.body.weight(.semibold)) }
                    // the other way in, for a link sent to the phone: smaller, under the scan
                    Button("Paste a link instead") {
                        model.pair(with: UIPasteboard.general.string ?? "")
                        if model.problem == nil { dismiss() }
                    }
                    .font(.footnote).foregroundStyle(.secondary)
                    if model.paired {
                        Text(model.computers.isEmpty ? "Paired" : "Paired: " + model.computers.map(\.name).joined(separator: ", ")).foregroundStyle(.secondary)
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
