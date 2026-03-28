// ContentView — Main content view with NavigationSplitView layout.
// Uses native NavigationSplitView for system-managed sidebar, titlebar, and background.

import SwiftUI
import ReaderModels
import ReaderBridge

struct ContentView: View {
    @EnvironmentObject var appState: AppState
    @EnvironmentObject var container: DIContainer
    @ObservedObject private var languageManager = LanguageManager.shared

    /// Whether the user is currently in reader mode.
    private var isReading: Bool {
        if case .reader = appState.currentDestination { return true }
        return false
    }

    private var splitViewVisibility: Binding<NavigationSplitViewVisibility> {
        Binding(
            get: {
                if isReading {
                    return .detailOnly
                }
                return appState.isSidebarVisible ? .all : .detailOnly
            },
            set: { newValue in
                guard !isReading else { return }
                appState.setLibrarySidebarVisible(newValue != .detailOnly)
            }
        )
    }

    var body: some View {
        GeometryReader { geometry in
            Group {
                if isReading {
                    // Reader mode: full-screen reader without sidebar
                    detailView
                } else {
                    // Library mode: native split view with sidebar
                    // Sidebar width is always 1/5 of the window width.
                    NavigationSplitView(columnVisibility: splitViewVisibility) {
                        SidebarView()
                            .navigationSplitViewColumnWidth(geometry.size.width / 5)
                    } detail: {
                        detailView
                    }
                    .navigationSplitViewStyle(.prominentDetail)
                }
            }
        }
        .frame(minWidth: 800, minHeight: 600)
        .animation(.easeInOut(duration: 0.25), value: isReading)
        .onOpenURL { url in
            handleOpenURL(url)
        }
        .alert(
            L("error.title"),
            isPresented: Binding(
                get: { appState.currentError != nil },
                set: { if !$0 { appState.currentError = nil } }
            ),
            presenting: appState.currentError
        ) { _ in
            Button(L("error.ok")) { appState.currentError = nil }
        } message: { error in
            Text(error.localizedDescription)
        }
    }

    @ViewBuilder
    private var detailView: some View {
        switch appState.currentDestination {
        case .library:
            LibraryView()
        case .reader(let bookId):
            if let book = appState.libraryBooks.first(where: { $0.id == bookId }),
               book.isPlainText {
                TxtReaderView(book: book, engine: container.engine)
            } else {
                ReaderView(bookId: bookId, engine: container.engine)
            }
        }
    }

    private func handleOpenURL(_ url: URL) {
        guard AppState.isSupportedFile(url) else { return }
        appState.importBook(from: url)
    }
}
