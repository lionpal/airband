import SwiftUI

/// Start a new agent from the phone: the coding tool, and the folder it works in. The folders his coding tools
/// have lately had sessions in are offered first; any other folder on the computer is reached by walking to
/// it. The agent is started there with nothing to do yet.
struct NewAgentSheet: View {
    @EnvironmentObject var model: RadioModel
    @Environment(\.dismiss) private var dismiss
    @State private var computer = ""
    @State private var tool = ""
    @State private var recent: [DeskRecent] = []
    @State private var loading = false
    @State private var place: Place?
    @State private var name = ""
    @State private var walking = false
    @State private var starting = false
    @State private var wrong: String?

    /// Where the agent is to work.
    struct Place: Equatable {
        var folder: String
        var shown: String
        /// A new folder is made inside it, called as the team is.
        var make = false
        /// A team works there already: the agent joins it, and its name is not his to change here.
        var team = false
        /// The team's name, as the computer has it or would give it there. Empty for a new folder: he names it.
        var name = ""
    }

    /// What the computer will call the team: the core names it by the computer's own rule.
    private var team: String { teamName(written: name) }
    /// Every choice made: a folder and a name, and the computer and the coding agent when there is more than one.
    private var ready: Bool { place != nil && !team.isEmpty && !starting && (model.view.stations.count <= 1 || !computer.isEmpty) && (model.usedTools.count <= 1 || !tool.isEmpty) }
    /// A folder he walked to, when it is none of the recent ones: shown with them, chosen.
    private var walked: Place? { place.flatMap { p in recent.contains { $0.folder == p.folder } && !p.make ? nil : p } }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if model.view.stations.count > 1 {
                        label("Computer")
                        chips(model.view.stations.map { ($0.id, $0.name, nil) }, chosen: computer) { computer = $0; (place, name) = (nil, ""); load() }
                    }
                    if model.usedTools.count > 1 {
                        label("Coding agent")
                        chips(model.usedTools.map { ($0.id, $0.name, "harness-\($0.id)") }, chosen: tool) { tool = $0 }
                    }
                    label("Recent folders")
                    VStack(spacing: 8) {
                        if loading && recent.isEmpty { ProgressView().tint(Palette.quiet).frame(maxWidth: .infinity, minHeight: 52) }
                        ForEach(recent.prefix(8), id: \.folder) { r in
                            row(r.name.isEmpty ? r.shown : r.name, r.shown, meta(r), chosen: place?.folder == r.folder && place?.make == false) {
                                place = Place(folder: r.folder, shown: r.shown, team: r.team)
                                name = r.name
                            }
                        }
                        if let walked {
                            row(walked.make ? "New folder" : (walked.shown as NSString).lastPathComponent, walked.make ? "Inside \(walked.shown)" : walked.shown, "", chosen: true) {}
                        }
                        if !loading && recent.isEmpty && walked == nil {
                            Text(computer.isEmpty ? "Pick a computer to see its folders." : "No coding agent has worked in a folder here yet.").font(.system(size: 15)).foregroundStyle(Palette.quiet)
                                .frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 6)
                        }
                        Button { walking = true } label: {
                            HStack(spacing: 12) {
                                Text("Choose another folder").font(.system(size: 16, weight: .semibold)).foregroundStyle(Palette.ink)
                                Spacer()
                                Image(systemName: "chevron.right").font(.system(size: 14, weight: .semibold)).foregroundStyle(Palette.dim)
                            }
                            .padding(.horizontal, 14).frame(minHeight: 52).background(Palette.card, in: RoundedRectangle(cornerRadius: 14)).contentShape(Rectangle())
                        }
                        .buttonStyle(.plain).disabled(computer.isEmpty).opacity(computer.isEmpty ? 0.4 : 1)
                    }
                    if let place {
                        label("Team")
                        if place.team {
                            Text(name).font(.system(size: 16)).foregroundStyle(Palette.soft).padding(.horizontal, 14)
                                .frame(maxWidth: .infinity, minHeight: 48, alignment: .leading).background(Palette.field, in: RoundedRectangle(cornerRadius: 12))
                        } else {
                            TextField("A short name, like shop", text: $name).textInputAutocapitalization(.never).autocorrectionDisabled()
                                .font(.system(size: 16)).foregroundStyle(Palette.ink).padding(.horizontal, 14)
                                .frame(minHeight: 48).background(Palette.field, in: RoundedRectangle(cornerRadius: 12))
                        }
                        Text(place.make ? (team.isEmpty ? "Name the team. The new folder is called the same." : "The agent starts in a new folder called \(name.trimmingCharacters(in: .whitespaces)), inside \(place.shown), with nothing to do yet. Its team is \(team). It waits for your first message.") : "The agent starts in \(place.shown) with nothing to do yet. It waits for your first message.")
                            .font(.system(size: 13)).foregroundStyle(Palette.quiet).lineSpacing(2)
                    }
                    if let wrong { Text(wrong).font(.system(size: 14)).foregroundStyle(.red) }
                }
                .padding(.horizontal, 20).padding(.top, 8).padding(.bottom, 20)
                .frame(maxWidth: 560).frame(maxWidth: .infinity)
            }
            .background(Palette.ground.ignoresSafeArea())
            .safeAreaInset(edge: .bottom, spacing: 0) {
                Button(action: start) {
                    Group { if starting { ProgressView().tint(Palette.ground) } else { Text("Start").font(.system(size: 18, weight: .semibold)) } }
                        .foregroundStyle(Palette.ground).frame(maxWidth: 520, minHeight: 56).frame(maxWidth: .infinity)
                        .background(Palette.ink.opacity(ready || starting ? 1 : 0.35), in: RoundedRectangle(cornerRadius: 18))
                }
                .buttonStyle(.plain).disabled(!ready).padding(.horizontal, 20).padding(.vertical, 12).frame(maxWidth: 560).frame(maxWidth: .infinity).background(Palette.ground)
            }
            .navigationTitle("New agent").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() }.foregroundStyle(Palette.soft) } }
            .toolbarBackground(Palette.ground, for: .navigationBar)
            .navigationDestination(isPresented: $walking) {
                FolderWalk(computer: computer) { chosen in
                    place = chosen
                    // a folder that is there gives the team its name; a new one is named by him
                    name = chosen.name
                    walking = false
                }
                .environmentObject(model)
            }
        }
        .preferredColorScheme(.dark)
        // on a wide screen, tall enough for the folders and the team's name together
        .presentationSizing(.page)
        .onAppear {
            // nothing is picked for him: the one there is, when there is only one; otherwise he picks
            computer = model.view.stations.count == 1 ? model.view.stations[0].id : ""
            tool = model.usedTools.count == 1 ? model.usedTools[0].id : ""
            load()
        }
        // the computers can become known after the sheet opens: the only one is taken, as above
        .onChange(of: model.view.stations) { _, stations in
            if computer.isEmpty, stations.count == 1 { computer = stations[0].id; load() }
        }
    }

    private func label(_ text: String) -> some View {
        Text(text).font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.quiet).padding(.top, 4)
    }

    /// A few choices side by side, one of them chosen: (what it is, what it is called, its mark).
    private func chips(_ all: [(String, String, String?)], chosen: String, pick: @escaping (String) -> Void) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(all, id: \.0) { id, called, mark in
                    let on = id == chosen
                    Button { pick(id) } label: {
                        HStack(spacing: 8) {
                            if let mark, UIImage(named: mark) != nil { Image(mark).resizable().scaledToFit().frame(width: 18, height: 18) }
                            Text(called).font(.system(size: 15, weight: .semibold))
                        }
                        .foregroundStyle(on ? Palette.ink : Palette.soft).padding(.horizontal, 14).frame(minHeight: 44)
                        .background(Palette.card, in: RoundedRectangle(cornerRadius: 12))
                        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(on ? Palette.ink : Palette.raised, lineWidth: 2))
                    }
                    .buttonStyle(.plain).accessibilityAddTraits(on ? .isSelected : [])
                }
            }
        }
    }

    private func row(_ title: String, _ path: String, _ meta: String, chosen: Bool, tap: @escaping () -> Void) -> some View {
        Button(action: tap) {
            HStack(spacing: 12) {
                Image(systemName: "folder").font(.system(size: 18)).foregroundStyle(chosen ? Palette.ink : Palette.quiet).frame(width: 24)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).font(.system(size: 16, weight: .semibold)).foregroundStyle(Palette.ink).lineLimit(1)
                    Text(path).font(.system(size: 13)).foregroundStyle(Palette.quiet).lineLimit(1).truncationMode(.head)
                }
                Spacer(minLength: 8)
                if !meta.isEmpty { Text(meta).font(.system(size: 13)).foregroundStyle(Palette.quiet).multilineTextAlignment(.trailing).fixedSize() }
            }
            .padding(.horizontal, 14).padding(.vertical, 11).frame(minHeight: 52)
            .background(Palette.card, in: RoundedRectangle(cornerRadius: 14))
            .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(chosen ? Palette.ink : Color.clear, lineWidth: 2))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain).accessibilityAddTraits(chosen ? .isSelected : [])
    }

    /// Which coding tools worked there, and how long ago.
    private func meta(_ r: DeskRecent) -> String {
        var lines: [String] = []
        if !r.tools.isEmpty { lines.append(r.tools.joined(separator: ", ")) }
        if r.at > 0 { lines.append(RelativeDateTimeFormatter().localizedString(for: Date(timeIntervalSince1970: Double(r.at) / 1000), relativeTo: Date())) }
        return lines.joined(separator: "\n")
    }

    private func load() {
        // with no computer picked there is nowhere to list folders from yet
        guard !computer.isEmpty else { (recent, loading, wrong) = ([], false, nil); return }
        (loading, wrong) = (true, nil)
        let asked = computer
        model.recentFolders(on: computer) { found, problem in
            guard asked == computer else { return }
            loading = false
            if let found { recent = found } else { (recent, wrong) = ([], problem) }
            #if DEBUG
            // development only: `-airband.devNewAgent pick` chooses the first folder, `walk` opens the walk to
            // another, `start` chooses the first and starts an agent there. For a screenshot.
            let dev = Dev.text("devNewAgent")
            if dev == "walk" { walking = true }
            if dev == "pick" || dev == "start", let first = recent.first {
                place = Place(folder: first.folder, shown: first.shown, team: first.team)
                name = first.name
                if dev == "start" { start() }
            }
            // `make` asks for a new folder in his home folder, called as the team is, and starts an agent there
            if dev == "make" {
                model.folders(on: computer, under: "") { home, _ in
                    guard let home else { return }
                    place = Place(folder: home.path, shown: home.shown, make: true)
                    name = "attic"
                    start()
                }
            }
            #endif
        }
    }

    private func start() {
        guard let place, ready else { return }
        (starting, wrong) = (true, nil)
        model.newTeam(place.make ? name : team, on: computer, folder: place.folder, make: place.make, tool: tool.isEmpty ? nil : tool) { made, problem in
            starting = false
            if let made {
                model.open(made: made)
                dismiss()
            } else {
                wrong = problem
            }
        }
    }
}

/// Walk a computer's folders to the one an agent should work in. It starts at his home folder and goes up from
/// there to the top of the disk, and into other disks.
struct FolderWalk: View {
    @EnvironmentObject var model: RadioModel
    let computer: String
    let use: (NewAgentSheet.Place) -> Void
    @State private var here: DeskFolders?
    @State private var makeNew = false
    @State private var loading = false
    @State private var wrong: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text(here?.shown ?? " ").font(.system(size: 15)).foregroundStyle(Palette.soft).lineLimit(2).truncationMode(.head)
                VStack(spacing: 0) {
                    if let here {
                        if let up = here.up {
                            line("arrow.up", "Up", dim: true) { open(up) }
                        }
                        ForEach(here.folders, id: \.path) { folder in
                            Rectangle().fill(Palette.raised).frame(height: 1)
                            line("folder", folder.name, dim: false) { open(folder.path) }
                        }
                        if here.folders.isEmpty {
                            Rectangle().fill(Palette.raised).frame(height: 1)
                            Text("No folders inside this one").font(.system(size: 15)).foregroundStyle(Palette.quiet).frame(maxWidth: .infinity, minHeight: 48, alignment: .leading).padding(.horizontal, 14)
                        }
                    } else if loading {
                        ProgressView().tint(Palette.quiet).frame(maxWidth: .infinity, minHeight: 96)
                    }
                }
                .background(Palette.card, in: RoundedRectangle(cornerRadius: 14)).clipShape(RoundedRectangle(cornerRadius: 14))
                Toggle("Make a new folder here", isOn: $makeNew).font(.system(size: 16)).foregroundStyle(Palette.ink).tint(Palette.signal)
                    .padding(.horizontal, 14).frame(minHeight: 52).background(Palette.card, in: RoundedRectangle(cornerRadius: 14))
                Text("You can go up from your home folder to the rest of the computer, and into other disks.").font(.system(size: 13)).foregroundStyle(Palette.quiet).lineSpacing(2)
                if let wrong { Text(wrong).font(.system(size: 14)).foregroundStyle(.red) }
            }
            .padding(.horizontal, 20).padding(.top, 8).padding(.bottom, 20)
            .frame(maxWidth: 560).frame(maxWidth: .infinity)
        }
        .background(Palette.ground.ignoresSafeArea())
        .safeAreaInset(edge: .bottom, spacing: 0) {
            Button {
                if let here { use(NewAgentSheet.Place(folder: here.path, shown: here.shown, make: makeNew, team: here.hasTeam && !makeNew, name: makeNew ? "" : here.team)) }
            } label: {
                Text(makeNew ? "New folder in \(here?.shown ?? "")" : "Use \(here?.shown ?? "")").font(.system(size: 18, weight: .semibold)).lineLimit(1).truncationMode(.head)
                    .foregroundStyle(Palette.ground).padding(.horizontal, 16).frame(maxWidth: 520, minHeight: 56).frame(maxWidth: .infinity)
                    .background(Palette.ink.opacity(here == nil ? 0.35 : 1), in: RoundedRectangle(cornerRadius: 18))
            }
            .buttonStyle(.plain).disabled(here == nil).padding(.horizontal, 20).padding(.vertical, 12).frame(maxWidth: 560).frame(maxWidth: .infinity).background(Palette.ground)
        }
        .navigationTitle("Choose a folder").navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(Palette.ground, for: .navigationBar)
        .onAppear { if here == nil { open("") } }
    }

    private func line(_ symbol: String, _ text: String, dim: Bool, tap: @escaping () -> Void) -> some View {
        Button(action: tap) {
            HStack(spacing: 12) {
                Image(systemName: symbol).font(.system(size: 17)).foregroundStyle(dim ? Palette.soft : Palette.quiet).frame(width: 24)
                Text(text).font(.system(size: 16)).foregroundStyle(dim ? Palette.soft : Palette.ink).lineLimit(1)
                Spacer()
                if !dim { Image(systemName: "chevron.right").font(.system(size: 14, weight: .semibold)).foregroundStyle(Palette.dim) }
            }
            .padding(.horizontal, 14).frame(minHeight: 48).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func open(_ path: String) {
        (loading, wrong) = (true, nil)
        model.folders(on: computer, under: path) { found, problem in
            loading = false
            if let found { here = found } else { wrong = problem }
        }
    }
}
