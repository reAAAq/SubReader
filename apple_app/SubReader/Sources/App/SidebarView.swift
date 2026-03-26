// SidebarView — Navigation sidebar with library and TOC sections.

import SwiftUI
import ReaderModels
import ReaderBridge

struct SidebarView: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject private var languageManager = LanguageManager.shared

    var body: some View {
        List {
            Section(L("sidebar.library")) {
                Button {
                    appState.currentDestination = .library
                } label: {
                    Label(L("sidebar.allBooks"), systemImage: "books.vertical")
                }
                .buttonStyle(.plain)
            }

            if appState.currentBookId != nil {
                Section(L("sidebar.reading")) {
                    Button {
                        NotificationCenter.default.post(name: .toggleTOC, object: nil)
                    } label: {
                        Label(L("sidebar.toc"), systemImage: "list.bullet")
                    }
                    .buttonStyle(.plain)

                    Button {
                        // Show bookmarks panel
                    } label: {
                        Label(L("sidebar.bookmarks"), systemImage: "bookmark")
                    }
                    .buttonStyle(.plain)

                    Button {
                        // Show annotations panel
                    } label: {
                        Label(L("sidebar.annotations"), systemImage: "highlighter")
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .listStyle(.sidebar)
        .frame(minWidth: 180)
    }
}
