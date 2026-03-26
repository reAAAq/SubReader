// AppCommands — macOS menu bar commands.

import SwiftUI
import UniformTypeIdentifiers
import ReaderBridge

struct AppCommands: Commands {
    @ObservedObject var appState: AppState
    @ObservedObject var languageManager = LanguageManager.shared

    var body: some Commands {
        // File menu
        CommandGroup(after: .newItem) {
            Button(L("commands.openFile")) {
                openFile()
            }
            .keyboardShortcut("o", modifiers: .command)
        }

        // View menu
        CommandGroup(after: .sidebar) {
            Button(L("commands.toggleSidebar")) {
                appState.isSidebarVisible.toggle()
            }
            .keyboardShortcut("t", modifiers: .command)

            Divider()

            Button(L("commands.bookmarkCurrentPage")) {
                NotificationCenter.default.post(name: .addBookmark, object: nil)
            }
            .keyboardShortcut("b", modifiers: .command)
        }
    }

    private func openFile() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [
            UTType(filenameExtension: "epub"),
            UTType.plainText,
        ].compactMap { $0 }
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false

        if panel.runModal() == .OK, let url = panel.url {
            if let data = try? Data(contentsOf: url) {
                appState.importBook(data: data, fileURL: url)
            }
        }
    }
}

// MARK: - Notification Names

extension Notification.Name {
    static let addBookmark = Notification.Name("com.subreader.addBookmark")
    static let toggleTOC = Notification.Name("com.subreader.toggleTOC")
}
