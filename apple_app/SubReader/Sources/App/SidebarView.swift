// SidebarView — Navigation sidebar using native List with .sidebar style.

import SwiftUI
import ReaderModels
import ReaderBridge

/// Sidebar items for List selection binding.
enum SidebarItem: Hashable {
    case allBooks
    case toc
    case bookmarks
    case annotations
}

struct SidebarView: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject private var languageManager = LanguageManager.shared

    @State private var selection: SidebarItem? = .allBooks

    var body: some View {
        List(selection: $selection) {
            Section(L("sidebar.library")) {
                Label(L("sidebar.allBooks"), systemImage: "books.vertical")
                    .tag(SidebarItem.allBooks)
            }

            if appState.isReaderActive {
                Section(L("sidebar.reading")) {
                    Label(L("sidebar.toc"), systemImage: "list.bullet")
                        .tag(SidebarItem.toc)

                    Label(L("sidebar.bookmarks"), systemImage: "bookmark")
                        .tag(SidebarItem.bookmarks)

                    Label(L("sidebar.annotations"), systemImage: "highlighter")
                        .tag(SidebarItem.annotations)
                }
            }
        }
        .listStyle(.sidebar)
        .searchable(text: $appState.searchText, placement: .sidebar, prompt: L("sidebar.search"))
        .onChange(of: selection) { _, newValue in
            handleSelection(newValue)
        }
        .onChange(of: appState.currentDestination) { _, newValue in
            // Sync selection when destination changes externally
            if case .library = newValue {
                selection = .allBooks
            } else if case .reader = newValue, selection == .allBooks {
                selection = .toc
            }
        }
    }

    private func handleSelection(_ item: SidebarItem?) {
        guard let item else { return }
        switch item {
        case .allBooks:
            appState.exitReader()
        case .toc:
            NotificationCenter.default.post(name: .toggleTOC, object: nil)
        case .bookmarks:
            // Show bookmarks panel
            break
        case .annotations:
            // Show annotations panel
            break
        }
    }
}
