// ContentView — Main content view with NavigationSplitView layout.

import SwiftUI
import ReaderModels
import ReaderBridge

struct ContentView: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject private var languageManager = LanguageManager.shared

    var body: some View {
        NavigationSplitView {
            SidebarView()
        } detail: {
            switch appState.currentDestination {
            case .library:
                LibraryView()
            case .reader(let bookId):
                if let book = appState.libraryBooks.first(where: { $0.id == bookId }),
                   book.isPlainText {
                    TxtReaderView(book: book)
                } else {
                    ReaderView(bookId: bookId)
                }
            }
        }
        .navigationSplitViewStyle(.balanced)
        .frame(minWidth: 800, minHeight: 600)
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

    private func handleOpenURL(_ url: URL) {
        guard AppState.isSupportedFile(url) else { return }
        guard let data = try? Data(contentsOf: url) else { return }
        appState.importBook(data: data, fileURL: url)
    }
}
