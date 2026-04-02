// SubReaderApp — Main application entry point.
//
// Initializes the Rust engine, sets up DI, and configures the window.

import SwiftUI
import AppKit
import ReaderModels
import ReaderBridge

private final class SubReaderAppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
    }
}

@main
struct SubReaderApp: App {

    @NSApplicationDelegateAdaptor(SubReaderAppDelegate.self) private var appDelegate
    @StateObject private var appState: AppState
    @StateObject private var container: DIContainer
    @ObservedObject private var languageManager = LanguageManager.shared
    @Environment(\.scenePhase) private var scenePhase

    init() {
        let di = DIContainer()

        // Initialize the Rust engine
        let dbPath = Self.databasePath()
        let deviceId = Self.deviceIdentifier()

        let result = di.engine.initialize(dbPath: dbPath, deviceId: deviceId, baseURL: Self.backendURL())
        switch result {
        case .success:
            break
        case .failure(let error):
            // Log but don't crash — the app can still show an error state
            print("⚠️ Engine init failed: \(error.localizedDescription)")
        }

        let state = AppState(
            engine: di.engine,
            chapterCache: di.chapterCache,
            coverCache: di.coverCache
        )
        _appState = StateObject(wrappedValue: state)
        _container = StateObject(wrappedValue: di)
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(appState)
                .environmentObject(container)
                .environmentObject(container.authService)
                .environmentObject(container.syncService)
                .environmentObject(languageManager)
                .background(WindowActivationView())
                .onChange(of: scenePhase) { _, newPhase in
                    switch newPhase {
                    case .active:
                        NSApp.activate(ignoringOtherApps: true)
                        if let keyWindow = NSApp.windows.first(where: { $0.canBecomeKey }) {
                            keyWindow.makeKeyAndOrderFront(nil)
                        }
                        if container.authService.isLoggedIn && container.syncService.autoSyncEnabled {
                            container.syncService.startScheduler()
                        }
                    case .background:
                        container.syncService.stopScheduler()
                    default:
                        break
                    }
                }
        }
        .windowToolbarStyle(.unified)
        .commands {
            AppCommands(appState: appState)
        }

        #if os(macOS)
        Window(L("settings.account"), id: "account") {
            AccountView()
                .environmentObject(appState)
                .environmentObject(container)
                .environmentObject(container.authService)
                .environmentObject(container.syncService)
                .environmentObject(languageManager)
                .frame(width: 340, height: 420)
        }
        .windowResizability(.contentSize)
        .windowStyle(.hiddenTitleBar)

        Settings {
            SettingsView()
                .environmentObject(appState)
                .environmentObject(container)
                .environmentObject(container.authService)
                .environmentObject(container.syncService)
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

    /// Backend API URL from UserDefaults or bundled configuration.
    private static func backendURL() -> String? {
        // Check UserDefaults first (allows runtime override)
        if let url = UserDefaults.standard.string(forKey: "com.subreader.backend-url"), !url.isEmpty {
            return url
        }
        // Default backend URL (can be overridden via Settings or launch arguments)
        #if DEBUG
        return "http://localhost:8080"
        #else
        return nil
        #endif
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

private struct WindowActivationView: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        DispatchQueue.main.async {
            if let window = view.window {
                window.makeKeyAndOrderFront(nil)
                window.makeFirstResponder(nil)
            }
        }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        DispatchQueue.main.async {
            if let window = nsView.window, !window.isKeyWindow {
                window.makeKeyAndOrderFront(nil)
            }
        }
    }
}
