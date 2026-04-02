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
    @EnvironmentObject var authService: AuthService
    @ObservedObject private var languageManager = LanguageManager.shared
    @Environment(\.openWindow) private var openWindow

    @State private var selection: SidebarItem? = .allBooks

    var body: some View {
        VStack(spacing: 0) {
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

            Divider()

            // MARK: - User Profile Button (Apple Books style)
            Button {
                openAccountWindow()
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: authService.isLoggedIn ? "person.crop.circle.fill" : "person.crop.circle")
                        .font(.system(size: 24))
                        .foregroundStyle(authService.isLoggedIn ? .primary : .secondary)

                    VStack(alignment: .leading, spacing: 1) {
                        if authService.isLoggedIn {
                            Text(L("sidebar.signedIn"))
                                .font(.callout)
                                .foregroundStyle(.primary)
                        } else {
                            Text(L("sidebar.signIn"))
                                .font(.callout)
                                .foregroundStyle(.secondary)
                        }
                    }

                    Spacer()
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
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

    // MARK: - Private Methods

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

    /// Open the account window.
    private func openAccountWindow() {
        openWindow(id: "account")
    }
}
