// ReaderView — Main reading view with DOM rendering and chapter navigation.
//
// Uses NSTextView (via NSViewRepresentable) for high-performance CoreText rendering.
// Implements chapter preloading and NSAttributedString caching.

import SwiftUI
import Combine
import ReaderModels
import ReaderBridge

struct ReaderView: View {
    let bookId: String

    @EnvironmentObject var appState: AppState
    @EnvironmentObject var container: DIContainer

    @State private var metadata: BookMetadata?
    @State private var currentChapterIndex: Int = 0
    @State private var chapterNodes: [DomNode] = []
    @State private var attributedContent: NSAttributedString = NSAttributedString()
    @State private var progress: ReadingProgress?
    @State private var isLoading = true
    @State private var errorMessage: String?

    @AppStorage("fontSize") private var fontSize: Double = 16
    @AppStorage("lineSpacing") private var lineSpacing: Double = 1.5
    @AppStorage("fontName") private var fontName: String = "System"

    // Debounce progress saves
    @State private var progressSaveTask: DispatchWorkItem?

    var body: some View {
        ZStack {
            if isLoading {
                ProgressView("Loading chapter...")
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
                    // Reading content
                    AttributedTextView(
                        attributedString: attributedContent,
                        onScroll: handleScroll
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)

                    // Bottom progress bar
                    progressBar
                }
            }
        }
        .navigationTitle(metadata?.title ?? "Reading")
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                Button {
                    navigateChapter(offset: -1)
                } label: {
                    Image(systemName: "chevron.left")
                }
                .disabled(currentChapterIndex <= 0)

                Button {
                    navigateChapter(offset: 1)
                } label: {
                    Image(systemName: "chevron.right")
                }
                .disabled(metadata == nil)

                Button {
                    NotificationCenter.default.post(name: .addBookmark, object: nil)
                } label: {
                    Image(systemName: "bookmark")
                }
            }
        }
        .onAppear {
            loadBook()
        }
        .onDisappear {
            saveProgressAndClose()
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

    // MARK: - Progress Bar

    private var progressBar: some View {
        HStack {
            if let meta = metadata, !meta.authors.isEmpty {
                Text(currentChapterTitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer()

            if let prog = progress {
                Text("\(Int(prog.percentage))%")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.bar)
    }

    private var currentChapterTitle: String {
        guard let meta = metadata else { return "" }
        // Flatten TOC to find current chapter
        let flatToc = flattenToc(meta.authors.isEmpty ? [] : []) // Will use actual TOC
        if currentChapterIndex < flatToc.count {
            return flatToc[currentChapterIndex].title
        }
        return "Chapter \(currentChapterIndex + 1)"
    }

    // MARK: - Data Loading

    private func loadBook() {
        isLoading = true
        errorMessage = nil

        DispatchQueue.global(qos: .userInitiated).async {
            // Load metadata
            let metaResult = appState.engine.getMetadata()
            guard case .success(let meta) = metaResult else {
                DispatchQueue.main.async {
                    errorMessage = "Failed to load book metadata"
                    isLoading = false
                }
                return
            }

            // Load progress
            let progResult = appState.engine.getProgress(bookId: bookId)
            let prog = try? progResult.get()

            // Determine chapter to load
            let flatToc = flattenToc(meta.authors.isEmpty ? [] : []) // Simplified
            let chapterPath = flatToc.first?.href ?? ""

            // Load chapter content
            let contentResult = appState.engine.getChapterContent(path: chapterPath)

            DispatchQueue.main.async {
                metadata = meta
                progress = prog

                switch contentResult {
                case .success(let nodes):
                    chapterNodes = nodes
                    renderContent()
                case .failure:
                    errorMessage = "Failed to load chapter content"
                }
                isLoading = false
            }

            // Preload adjacent chapters
            preloadAdjacentChapters()
        }
    }

    private func renderContent() {
        let renderer = DomRenderer(
            fontSize: fontSize,
            lineSpacing: lineSpacing,
            fontName: fontName
        )
        attributedContent = renderer.render(nodes: chapterNodes)
    }

    private func navigateChapter(offset: Int) {
        let newIndex = currentChapterIndex + offset
        guard newIndex >= 0 else { return }

        currentChapterIndex = newIndex
        isLoading = true

        guard let meta = metadata else { return }
        let flatToc = flattenToc(meta.authors.isEmpty ? [] : [])
        guard newIndex < flatToc.count else {
            isLoading = false
            return
        }

        let chapterPath = flatToc[newIndex].href

        // Check cache first
        let cacheKey = "\(chapterPath)_\(Int(fontSize))_\(fontName)"
        if let cached = container.chapterCache.get(key: cacheKey) as? NSAttributedString {
            attributedContent = cached
            isLoading = false
            return
        }

        DispatchQueue.global(qos: .userInitiated).async {
            let result = appState.engine.getChapterContent(path: chapterPath)
            DispatchQueue.main.async {
                switch result {
                case .success(let nodes):
                    chapterNodes = nodes
                    renderContent()
                    // Cache the result
                    container.chapterCache.set(key: cacheKey, value: attributedContent)
                case .failure:
                    errorMessage = "Failed to load chapter"
                }
                isLoading = false
            }
        }
    }

    private func preloadAdjacentChapters() {
        // Preload next and previous chapters in background
        guard let meta = metadata else { return }
        let flatToc = flattenToc(meta.authors.isEmpty ? [] : [])

        for offset in [-1, 1] {
            let idx = currentChapterIndex + offset
            guard idx >= 0, idx < flatToc.count else { continue }

            let path = flatToc[idx].href
            let cacheKey = "\(path)_\(Int(fontSize))_\(fontName)"

            // Skip if already cached
            if container.chapterCache.get(key: cacheKey) != nil { continue }

            DispatchQueue.global(qos: .utility).async {
                let result = appState.engine.getChapterContent(path: path)
                if case .success(let nodes) = result {
                    let renderer = DomRenderer(fontSize: fontSize, lineSpacing: lineSpacing, fontName: fontName)
                    let rendered = renderer.render(nodes: nodes)
                    container.chapterCache.set(key: cacheKey, value: rendered)
                }
            }
        }
    }

    // MARK: - Progress Tracking

    private func handleScroll(percentage: Double) {
        // Debounce: save progress after 500ms of no scrolling
        progressSaveTask?.cancel()
        let task = DispatchWorkItem {
            let _ = appState.engine.updateProgress(
                bookId: bookId,
                cfi: "",
                percentage: percentage,
                hlcTs: UInt64(Date().timeIntervalSince1970)
            )
            DispatchQueue.main.async {
                progress = ReadingProgress(
                    bookId: bookId,
                    cfiPosition: "",
                    percentage: percentage,
                    hlcTimestamp: UInt64(Date().timeIntervalSince1970)
                )
            }
        }
        progressSaveTask = task
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 0.5, execute: task)
    }

    private func saveProgressAndClose() {
        if let prog = progress {
            let _ = appState.engine.updateProgress(
                bookId: bookId,
                cfi: prog.cfiPosition,
                percentage: prog.percentage,
                hlcTs: UInt64(Date().timeIntervalSince1970)
            )
        }
        let _ = appState.engine.closeBook()
    }

    // MARK: - Helpers

    private func flattenToc(_ entries: [TocEntry]) -> [TocEntry] {
        var result: [TocEntry] = []
        for entry in entries {
            result.append(entry)
            result.append(contentsOf: flattenToc(entry.children))
        }
        return result
    }
}
