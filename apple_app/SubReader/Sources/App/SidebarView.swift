// SidebarView — Navigation sidebar with library and TOC sections.

import SwiftUI
import ReaderModels
import ReaderBridge

struct SidebarView: View {
    @EnvironmentObject var appState: AppState

    var body: some View {
        List {
            Section("Library") {
                Button {
                    appState.currentDestination = .library
                } label: {
                    Label("All Books", systemImage: "books.vertical")
                }
                .buttonStyle(.plain)
            }

            if appState.currentBookId != nil {
                Section("Reading") {
                    Button {
                        NotificationCenter.default.post(name: .toggleTOC, object: nil)
                    } label: {
                        Label("Table of Contents", systemImage: "list.bullet")
                    }
                    .buttonStyle(.plain)

                    Button {
                        // Show bookmarks panel
                    } label: {
                        Label("Bookmarks", systemImage: "bookmark")
                    }
                    .buttonStyle(.plain)

                    Button {
                        // Show annotations panel
                    } label: {
                        Label("Annotations", systemImage: "highlighter")
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .listStyle(.sidebar)
        .frame(minWidth: 180)
    }
}
