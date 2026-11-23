// How the model that writes his summaries is told to write them: the whole text, his to change. The core keeps
// it and sends it to his computers; the rest of what that model is told is fixed there.
import SwiftUI

struct SummaryBriefPage: View {
    @EnvironmentObject var model: RadioModel
    @State private var text = ""
    @State private var kept = ""
    @State private var wrong: String?
    @State private var resetting = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            TextEditor(text: $text).font(.system(size: 15)).scrollContentBackground(.hidden)
                .padding(10).background(Palette.field, in: RoundedRectangle(cornerRadius: 12))
            if let wrong { Text(wrong).font(.system(size: 13)).foregroundStyle(.red) }
        }
        .padding(.horizontal, 20).padding(.vertical, 12).frame(maxWidth: 560).frame(maxWidth: .infinity)
        .navigationTitle("Summary instructions").navigationBarTitleDisplayMode(.inline)
        .darkPage()
        .toolbar {
            ToolbarItem(placement: .confirmationAction) { Button("Save", action: save).disabled(text == kept) }
            ToolbarItem(placement: .bottomBar) { Button("Reset") { resetting = true } }
        }
        .confirmationDialog("Go back to the usual instructions?", isPresented: $resetting, titleVisibility: .visible) {
            Button("Reset", role: .destructive) {
                model.desk.resetSummaryBrief()
                load()
            }
            Button("Cancel", role: .cancel) {}
        }
        .onAppear(perform: load)
    }

    private func load() {
        (text, kept, wrong) = (model.desk.summaryBrief(), model.desk.summaryBrief(), nil)
    }

    private func save() {
        do {
            try model.desk.setSummaryBrief(text: text)
            load()
        } catch {
            wrong = RadioModel.words(for: error, else: "That didn't save. Try again.")
        }
    }
}
