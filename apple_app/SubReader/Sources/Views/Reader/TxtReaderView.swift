// TxtReaderView — Reading view for plain-text files.
//
// Renders TXT content parsed by the Rust engine.
// Supports chapter navigation, scroll progress tracking, and theme settings.

import SwiftUI
import ReaderModels
import ReaderBridge

struct TxtReaderView: View {
    let book: LibraryBook

    @EnvironmentObject var appState: AppState
    @EnvironmentObject var container: DIContainer
    @ObservedObject private var languageManager = LanguageManager.shared

    @State private var parseResult: TxtParseResult?
    @State private var currentChapterIndex: Int = 0
    @State private var attributedContent: NSAttributedString = NSAttributedString()
    @State private var isLoading = true
    @State private var errorMessage: String?
    @State private var scrollPercentage: Double = 0
    /// Whether the TOC sidebar is visible.
    @State private var showTOC: Bool = false

    @AppStorage("fontSize") private var fontSize: Double = 16
    @AppStorage("lineSpacing") private var lineSpacing: Double = 1.5
    @AppStorage("fontName") private var fontName: String = "System"

    /// Current chapter href for TOC highlighting (uses index as string).
    private var currentChapterHref: String? {
        return "\(currentChapterIndex)"
    }

    /// Convert TxtChapters to TocEntry format for TOCSidebarView.
    private var tocEntries: [TocEntry] {
        guard let parseResult else { return [] }
        return parseResult.chapters.enumerated().map { index, chapter in
            TocEntry(title: chapter.title, href: "\(index)", level: 0, children: [])
        }
    }

    var body: some View {
        HStack(spacing: 0) {
            // TOC sidebar
            if showTOC {
                TOCSidebarView(
                    tocEntries: tocEntries,
                    currentChapterHref: currentChapterHref,
                    onSelectChapter: { entry in
                        if let idx = Int(entry.href), idx != currentChapterIndex {
                            currentChapterIndex = idx
                            renderCurrentChapter()
                        }
                    }
                )
                .frame(width: 260)
                .transition(.move(edge: .leading))

                Divider()
            }

            // Main content
            ZStack {
                if isLoading {
                    ProgressView(L("reader.loading"))
                } else if let error = errorMessage {
                    VStack(spacing: 12) {
                        Image(systemName: "exclamationmark.triangle")
                            .font(.largeTitle)
                            .foregroundStyle(.secondary)
                        Text(error)
                            .foregroundStyle(.secondary)
                    }
                } else {
                    VStack(spacing: 0) {
                        AttributedTextView(
                            attributedString: attributedContent,
                            onScroll: { pct in scrollPercentage = pct }
                        )
                        .frame(maxWidth: .infinity, maxHeight: .infinity)

                        bottomBar
                    }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .animation(.easeInOut(duration: 0.2), value: showTOC)
        .navigationTitle(book.metadata.title)
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                Button {
                    navigateChapter(offset: -1)
                } label: {
                    Image(systemName: "chevron.left")
                }
                .disabled(currentChapterIndex <= 0)

                Text(chapterIndicator)
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Button {
                    navigateChapter(offset: 1)
                } label: {
                    Image(systemName: "chevron.right")
                }
                .disabled(parseResult == nil || currentChapterIndex >= (parseResult?.chapters.count ?? 1) - 1)
            }
        }
        .onAppear {
            loadContent()
        }
        .onReceive(NotificationCenter.default.publisher(for: .toggleTOC)) { _ in
            withAnimation {
                showTOC.toggle()
            }
        }
        .onKeyPress(.leftArrow) {
            navigateChapter(offset: -1)
            return .handled
        }
        .onKeyPress(.rightArrow) {
            navigateChapter(offset: 1)
            return .handled
        }
    }

    // MARK: - Bottom Bar

    private var bottomBar: some View {
        HStack {
            if let parseResult, !parseResult.chapters.isEmpty {
                Text(parseResult.chapters[currentChapterIndex].title)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer()

            Text(overallProgress)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.bar)
    }

    private var chapterIndicator: String {
        guard let parseResult else { return "" }
        return "\(currentChapterIndex + 1)/\(parseResult.chapters.count)"
    }

    private var overallProgress: String {
        guard let parseResult, !parseResult.chapters.isEmpty else { return "0%" }
        let chapterWeight = 1.0 / Double(parseResult.chapters.count)
        let pct = (Double(currentChapterIndex) + scrollPercentage / 100.0) * chapterWeight * 100.0
        return "\(Int(pct))%"
    }

    // MARK: - Loading

    private func loadContent() {
        isLoading = true
        errorMessage = nil

        let engine = container.engine
        let bookId = book.id
        let fileURL = book.fileURL

        DispatchQueue.global(qos: .userInitiated).async {
            let loaded = TxtContentStore.shared.loadIfNeeded(
                bookId: bookId,
                fileURL: fileURL,
                engine: engine
            )

            DispatchQueue.main.async {
                guard let loaded else {
                    errorMessage = L("reader.failedLoadTxt")
                    isLoading = false
                    return
                }

                parseResult = loaded
                renderCurrentChapter()
                isLoading = false
            }
        }
    }

    private func renderCurrentChapter() {
        guard let parseResult, currentChapterIndex < parseResult.chapters.count else { return }

        let chapter = parseResult.chapters[currentChapterIndex]
        let renderer = DomRenderer(
            fontSize: fontSize,
            lineSpacing: lineSpacing,
            fontName: fontName
        )

        // Rust already provides DomNodes — render them directly
        var nodes = chapter.nodes

        // Prepend chapter title as heading node
        let titleNode = DomNode(
            nodeType: .heading(level: 2),
            children: [DomNode(nodeType: .text, text: chapter.title)]
        )
        nodes.insert(titleNode, at: 0)

        attributedContent = renderer.render(nodes: nodes)
    }

    private func navigateChapter(offset: Int) {
        guard let parseResult else { return }
        let newIndex = currentChapterIndex + offset
        guard newIndex >= 0, newIndex < parseResult.chapters.count else { return }

        currentChapterIndex = newIndex
        renderCurrentChapter()
    }
}
