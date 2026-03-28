// LibraryView — Book library with Apple Books-style grid layout, import, and management.

import SwiftUI
import UniformTypeIdentifiers
import ReaderModels
import ReaderBridge

struct LibraryView: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject private var languageManager = LanguageManager.shared

    var body: some View {
        Group {
            if appState.libraryBooks.isEmpty {
                emptyStateView
            } else {
                gridView
            }
        }
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                importButton
            }
        }
        .toolbarBackground(.hidden, for: .windowToolbar)
        .navigationTitle("")
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

    // MARK: - Grid View (Apple Books Style)

    private var gridView: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                // Large title header
                VStack(alignment: .leading, spacing: 16) {
                    Text(L("library.title"))
                        .font(.largeTitle)
                        .fontWeight(.bold)
                        .padding(.horizontal, 24)
                        .padding(.top, 20)
                    
                    Divider()
                        .padding(.horizontal, 24)
                }
                .padding(.bottom, 24)

                // Book grid
                LazyVGrid(
                    columns: Array(
                        repeating: GridItem(.flexible(), spacing: 32, alignment: .top),
                        count: 4
                    ),
                    spacing: 48
                ) {
                    ForEach(appState.filteredBooks) { book in
                        BookCardView(book: book)
                            .frame(maxWidth: .infinity)
                            .onTapGesture(count: 2) {
                                appState.openBook(book)
                            }
                            .contextMenu {
                                bookContextMenu(book)
                            }
                    }
                }
                .padding(.horizontal, 40)

                // Bottom status bar
                HStack {
                    Spacer()
                    Text(L("library.bookCount", appState.filteredBooks.count))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                }
                .padding(.horizontal, 24)
                .padding(.vertical, 16)
            }
        }
    }

    // MARK: - Components

    private var importButton: some View {
        Button {
            openFilePicker()
        } label: {
            Image(systemName: "ellipsis.circle")
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
                appState.importBook(from: url)
            }
        }
    }

    private func handleDrop(_ providers: [NSItemProvider]) {
        for provider in providers {
            provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { item, _ in
                guard let data = item as? Data,
                      let url = URL(dataRepresentation: data, relativeTo: nil),
                      AppState.isSupportedFile(url) else { return }

                DispatchQueue.main.async {
                    appState.importBook(from: url)
                }
            }
        }
    }
}

// MARK: - Book Card View (Apple Books Style)

struct BookCardView: View {
    let book: LibraryBook
    @EnvironmentObject var appState: AppState

    private let coverWidth: CGFloat = 160
    private let coverAspectRatio: CGFloat = 2.0 / 3.0

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            // Cover
            ZStack(alignment: .leading) {
                Group {
                    if let coverData = book.coverData, let nsImage = NSImage(data: coverData) {
                        // Real cover image
                        Image(nsImage: nsImage)
                            .resizable()
                            .aspectRatio(coverAspectRatio, contentMode: .fill)
                    } else {
                        // Generated cover (Apple Books style)
                        generatedCover
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: 4))

                // Spine shadow line on the left edge
                Rectangle()
                    .fill(
                        LinearGradient(
                            colors: [
                                .black.opacity(0.4),
                                .black.opacity(0.15),
                                .clear
                            ],
                            startPoint: .leading,
                            endPoint: .trailing
                        )
                    )
                    .frame(width: 6)
                    .clipShape(RoundedRectangle(cornerRadius: 4))
            }
            .shadow(color: .black.opacity(0.3), radius: 6, x: 3, y: 4)
            .shadow(color: .black.opacity(0.1), radius: 2, x: 1, y: 1)

            // Info row below cover: progress/new label + more button
            HStack(alignment: .center) {
                if book.progress == 0 {
                    // "New" badge
                    Text(L("library.new"))
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(.white)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3)
                        .background(Color.blue, in: Capsule())
                } else {
                    // Progress percentage
                    Text("\(Int(book.progress))%")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }

                Spacer()

                // More button (•••)
                Menu {
                    Button(L("library.open")) {
                        appState.openBook(book)
                    }
                    Divider()
                    Button(L("library.delete"), role: .destructive) {
                        appState.removeBook(id: book.id)
                    }
                } label: {
                    Image(systemName: "ellipsis")
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .frame(width: 20, height: 20)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .frame(width: coverWidth)
    }

    // MARK: - Generated Cover (Apple Books Style)

    private var generatedCover: some View {
        RoundedRectangle(cornerRadius: 4)
            .fill(
                LinearGradient(
                    colors: [
                        Color(red: 0.35, green: 0.35, blue: 0.35), // Lighter gray
                        Color(red: 0.15, green: 0.15, blue: 0.15)  // Darker gray
                    ],
                    startPoint: .top,
                    endPoint: .bottom
                )
            )
            .aspectRatio(coverAspectRatio, contentMode: .fill)
            .overlay {
                // Inner border
                RoundedRectangle(cornerRadius: 4)
                    .strokeBorder(.white.opacity(0.15), lineWidth: 0.5)
            }
            .overlay {
                VStack(spacing: 0) {
                    Spacer()
                        .frame(maxHeight: .infinity)

                    // Book title at upper 1/3
                    Text(book.metadata.title)
                        .font(.system(size: 22, weight: .bold))
                        .foregroundColor(.white)
                        .multilineTextAlignment(.center)
                        .lineLimit(3)
                        .padding(.horizontal, 16)

                    Spacer()
                        .frame(maxHeight: .infinity)
                    Spacer()
                        .frame(maxHeight: .infinity)

                    // Author name at lower 1/4
                    Text(book.metadata.authors.joined(separator: ", "))
                        .font(.system(size: 13))
                        .foregroundColor(.white.opacity(0.6))
                        .multilineTextAlignment(.center)
                        .lineLimit(2)
                        .padding(.horizontal, 16)

                    Spacer()
                        .frame(maxHeight: .infinity)
                }
                .padding(.vertical, 12)
            }
    }
}
