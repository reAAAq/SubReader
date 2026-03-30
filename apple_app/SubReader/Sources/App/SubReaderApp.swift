// SubReaderApp — Main application entry point.
//
// Initializes the Rust engine, sets up DI, and configures the window.

import SwiftUI
import ReaderModels
import ReaderBridge

@main
struct SubReaderApp: App {

    @StateObject private var appState: AppState
    @StateObject private var container: DIContainer
    @ObservedObject private var languageManager = LanguageManager.shared

    init() {
        let di = DIContainer()

        // Initialize the Rust engine
        let dbPath = Self.databasePath()
        let deviceId = Self.deviceIdentifier()

        let result = di.engine.initialize(dbPath: dbPath, deviceId: deviceId)
        switch result {
        case .success:
            break
        case .failure(let error):
            // Log but don't crash — the app can still show an error state
            print("⚠️ Engine init failed: \(error.localizedDescription)")
        }

        let state = AppState(engine: di.engine)
        _appState = StateObject(wrappedValue: state)
        _container = StateObject(wrappedValue: di)
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(appState)
                .environmentObject(container)
                .environmentObject(languageManager)
        }
        .windowToolbarStyle(.unified)
        .commands {
            AppCommands(appState: appState)
        }

        #if os(macOS)
        Settings {
            SettingsView()
                .environmentObject(appState)
                .environmentObject(languageManager)
        }
        #endif
    }

    // MARK: - Private Helpers

    /// Database path: ~/Library/Application Support/SubReader/reader.db
    private static func databasePath() -> String {
        let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let subReaderDir = appSupport.appendingPathComponent("SubReader", isDirectory: true)
        try? FileManager.default.createDirectory(at: subReaderDir, withIntermediateDirectories: true)
        return subReaderDir.appendingPathComponent("reader.db").path
    }

    /// Stable device identifier persisted in UserDefaults.
    private static func deviceIdentifier() -> String {
        let key = "com.subreader.device-id"
        if let existing = UserDefaults.standard.string(forKey: key) {
            return existing
        }
        let newId = UUID().uuidString
        UserDefaults.standard.set(newId, forKey: key)
        return newId
    }
}
