import AppIntents
import SwiftUI

// One screen: a feed. Everything his agents report or ask arrives in one stream, newest at the bottom, with what
// he said back. Above it, his teams and what each is doing. Below it, the talk button, with quick messages to its
// left and the keyboard to its right. The design is the one on airband.dev.

@main
struct AirbandApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var model = RadioModel.shared

    var body: some Scene {
        WindowGroup {
            FeedView()
                .environmentObject(model)
                .onOpenURL { model.open($0) }
                .tint(Palette.signal)
                .preferredColorScheme(.dark)
        }
    }
}
/// "Hey Siri, talk to Airband", the Action button, or a Shortcut: open the microphone.
struct TalkIntent: AppIntent {
    static var title: LocalizedStringResource = "Talk to airband"
    static var description = IntentDescription("Opens the microphone so you can speak to your agents.")
    static var openAppWhenRun = false

    @MainActor func perform() async throws -> some IntentResult {
        await RadioModel.shared.talk()
        return .result()
    }
}
struct AirbandShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: TalkIntent(), phrases: ["Talk to \(.applicationName)", "\(.applicationName) radio"], shortTitle: "Talk", systemImageName: "dot.radiowaves.left.and.right")
    }
}
