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

    /// The job whose page was asked for at launch, in development only.
    private var devJob: ChoiceJob? { model.jobs.first { $0.title == devPage } }

    private func choicePage(_ job: ChoiceJob) -> some View {
        ChoicePage(title: job.title, headline: job.headline) { ChoicePicker(job: job, mode: .settings) }.environmentObject(model)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    // one row for each job the core lists, opening the same picker the first run showed
                    ForEach(model.jobs, id: \.self) { job in
                        NavigationLink {
                            choicePage(job)
                        } label: {
                            row(job.title, model.chosenLabel(job))
                        }
                    }
                    NavigationLink("Summary instructions") { SummaryBriefPage().environmentObject(model) }
                }
                Section {
                    NavigationLink {
                        ChoicePage(title: "Coding agents", headline: Headline.tools) {
                            VStack(alignment: .leading, spacing: 14) {
                                Text("Check the ones you use.").font(.system(size: 15)).foregroundStyle(Palette.soft)
                                ToolsPicker(single: false)
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
                    Text("Messages read aloud stay short.")
                }
                if !model.computers.isEmpty {
                    Section {
                        if model.view.severalComputers {
                            Picker("Default computer", selection: Binding(get: { model.view.mainComputer ?? "" }, set: { model.setMainComputer($0) })) {
                                if model.view.mainComputer == nil { Text("Not chosen").tag("") }
                                ForEach(model.computers, id: \.self) { c in Label(c.name, systemImage: RadioModel.symbol(c.shape)).tag(c.id) }
                            }
                        }
                        ForEach(model.computers, id: \.id) { c in
                            Label(c.name, systemImage: RadioModel.symbol(c.shape))
                        }
                    } header: {
                        Text("Computers")
                    } footer: {
                        if model.view.severalComputers { Text(RadioModel.mainComputerNote) }
                    }
                }
                if model.paired {
                    Section {
                        NavigationLink("Connection") { ConnectionPage().environmentObject(model) }
                        NavigationLink("Accounts") { AccountsView().environmentObject(model) }
                        if push.denied {
                            Button("Turn on notifications") { UIApplication.shared.open(URL(string: UIApplication.openNotificationSettingsURLString)!) }
                        }
                    }
                }
                Section(footer: Text("Run ") + Text("airband pair").font(.system(.footnote, design: .monospaced)) + Text(" on a computer, then scan its code. Keep the link private.")) {
                    Button { scanning = true } label: { Label("Scan code", systemImage: "qrcode.viewfinder").font(.body.weight(.semibold)) }
                    // the other way in, for a link sent to the phone: smaller, under the scan
                    Button("Paste a link instead") {
                        if model.pairPasted() == nil { dismiss() }
                    }
                    .font(.footnote).foregroundStyle(.secondary)
                    if model.paired {
                        Text(model.computers.isEmpty ? "Paired" : "Paired: " + model.computers.map(\.name).joined(separator: ", ")).foregroundStyle(.secondary)
                        Button("Forget all", role: .destructive) { model.forgetAll() }
                    }
                    if let problem = model.problem { Text(problem).foregroundStyle(.red) }
                }
                Section {
                    ContactRow(from: .settings, plain: true)
                }
                Section {
                    NavigationLink("Acknowledgments") {
                        List(acknowledgments(), id: \.self) { Text($0) }.navigationTitle("Acknowledgments").navigationBarTitleDisplayMode(.inline)
                    }
                }
            }
            .fullScreenCover(isPresented: $scanning) { ScanSheet().environmentObject(model) }
            // development only: a page opened at launch for a screenshot
            .navigationDestination(isPresented: Binding(get: { devJob != nil }, set: { if !$0 { devPage = nil } })) {
                if let job = devJob { choicePage(job) }
            }
            .navigationDestination(isPresented: $showAccounts) { AccountsView().environmentObject(model) }
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
