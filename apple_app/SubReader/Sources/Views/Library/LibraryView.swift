// LibraryView — Book library with grid/list layout, import, and management.

import SwiftUI
import UniformTypeIdentifiers
import ReaderModels
import ReaderBridge

struct LibraryView: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject private var languageManager = LanguageManager.shared
    @AppStorage("libraryViewMode") private var viewMode: ViewMode = .grid

    enum ViewMode: String {
        case grid, list
    }

    var body: some View {
        Group {
            if appState.libraryBooks.isEmpty {
                emptyStateView
            } else {
                switch viewMode {
                case .grid:
                    gridView
                case .list:
                    listView
                }
            }
        }
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                viewModeToggle
                importButton
            }
        }
        .navigationTitle(L("library.title"))
        .onDrop(of: [.fileURL], isTargeted: nil) { providers in
            handleDrop(providers)
            return true
        }
        .overlay {
            if appState.isLoading {
                ProgressView(L("library.importing"))
                    .padding()
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
            }
        }
    }

    // MARK: - Empty State

    private var emptyStateView: some View {
        VStack(spacing: 16) {
            Image(systemName: "book.closed")
                .font(.system(size: 64))
                .foregroundStyle(.secondary)
            Text(L("library.noBooks"))
                .font(.title2)
                .fontWeight(.semibold)
            Text(L("library.dragHint"))
                .foregroundStyle(.secondary)
            Button(L("library.openFile")) {
                openFilePicker()
            }
            .buttonStyle(.borderedProminent)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Grid View

    private var gridView: some View {
        ScrollView {
            LazyVGrid(
                columns: [GridItem(.adaptive(minimum: 160, maximum: 200), spacing: 20)],
                spacing: 20
            ) {
                ForEach(appState.libraryBooks) { book in
                    BookCardView(book: book)
                        .onTapGesture(count: 2) {
                            appState.openBook(book)
                        }
                        .contextMenu {
                            bookContextMenu(book)
                        }
                }
            }
            .padding()
        }
    }

    // MARK: - List View

    private var listView: some View {
        List(appState.libraryBooks) { book in
            HStack(spacing: 12) {
                bookCoverThumbnail(book)
                    .frame(width: 40, height: 56)

                VStack(alignment: .leading, spacing: 4) {
                    Text(book.metadata.title)
                        .font(.headline)
                        .lineLimit(1)
                    Text(book.metadata.authors.joined(separator: ", "))
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer()

                ProgressView(value: book.progress, total: 100)
                    .frame(width: 80)
            }
            .padding(.vertical, 4)
            .onTapGesture(count: 2) {
                appState.openBook(book)
            }
            .contextMenu {
                bookContextMenu(book)
            }
        }
    }

    // MARK: - Components

    private var viewModeToggle: some View {
        Picker(L("library.view"), selection: $viewMode) {
            Image(systemName: "square.grid.2x2")
                .tag(ViewMode.grid)
            Image(systemName: "list.bullet")
                .tag(ViewMode.list)
        }
        .pickerStyle(.segmented)
        .frame(width: 80)
    }

    private var importButton: some View {
        Button {
            openFilePicker()
        } label: {
            Image(systemName: "plus")
        }
        .keyboardShortcut("o", modifiers: .command)
    }

    @ViewBuilder
    private func bookContextMenu(_ book: LibraryBook) -> some View {
        Button(L("library.open")) {
            appState.openBook(book)
        }
        Divider()
        Button(L("library.delete"), role: .destructive) {
            appState.removeBook(id: book.id)
        }
    }

    @ViewBuilder
    private func bookCoverThumbnail(_ book: LibraryBook) -> some View {
        if let coverData = book.coverData, let nsImage = NSImage(data: coverData) {
            Image(nsImage: nsImage)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .clipShape(RoundedRectangle(cornerRadius: 4))
        } else {
            RoundedRectangle(cornerRadius: 4)
                .fill(.quaternary)
                .overlay {
                    Image(systemName: "book.closed")
                        .foregroundStyle(.secondary)
                }
        }
    }

    // MARK: - Actions

    private func openFilePicker() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [
            UTType(filenameExtension: "epub"),
            UTType.plainText,
        ].compactMap { $0 }
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false

        if panel.runModal() == .OK {
            for url in panel.urls {
                if let data = try? Data(contentsOf: url) {
                    appState.importBook(data: data, fileURL: url)
                }
            }
        }
    }

    private func handleDrop(_ providers: [NSItemProvider]) {
        for provider in providers {
            provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { item, _ in
                guard let data = item as? Data,
                      let url = URL(dataRepresentation: data, relativeTo: nil),
                      AppState.isSupportedFile(url) else { return }

                if let fileData = try? Data(contentsOf: url) {
                    DispatchQueue.main.async {
                        appState.importBook(data: fileData, fileURL: url)
                    }
                }
            }
        }
    }
}

// MARK: - Book Card View

struct BookCardView: View {
    let book: LibraryBook

    var body: some View {
        VStack(spacing: 8) {
            // Cover
            Group {
                if let coverData = book.coverData, let nsImage = NSImage(data: coverData) {
                    Image(nsImage: nsImage)
                        .resizable()
                        .aspectRatio(2/3, contentMode: .fill)
                } else {
                    RoundedRectangle(cornerRadius: 8)
                        .fill(
                            LinearGradient(
                                colors: [.blue.opacity(0.3), .purple.opacity(0.3)],
                                startPoint: .topLeading,
                                endPoint: .bottomTrailing
                            )
                        )
                        .aspectRatio(2/3, contentMode: .fill)
                        .overlay {
                            VStack {
                                Image(systemName: "book.closed")
                                    .font(.largeTitle)
                                    .foregroundStyle(.secondary)
                                Text(book.metadata.title)
                                    .font(.caption)
                                    .multilineTextAlignment(.center)
                                    .foregroundStyle(.secondary)
                                    .padding(.horizontal, 8)
                            }
                        }
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .shadow(color: .black.opacity(0.15), radius: 4, y: 2)

            // Title & Author
            VStack(spacing: 2) {
                Text(book.metadata.title)
                    .font(.caption)
                    .fontWeight(.medium)
                    .lineLimit(2)
                    .multilineTextAlignment(.center)

                Text(book.metadata.authors.joined(separator: ", "))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            // Progress bar
            if book.progress > 0 {
                ProgressView(value: book.progress, total: 100)
                    .tint(.accentColor)
            }
        }
        .frame(width: 160)
        .padding(8)
    }
}
