// TxtReaderView — Reading view for plain-text files.
//
// Renders TXT content parsed by the Rust engine.
// Uses PaginatedReaderView for Apple Books-style paginated reading.

import SwiftUI
import ReaderModels
import ReaderBridge

struct TxtReaderView: View {
    let book: LibraryBook

    @EnvironmentObject var appState: AppState
    @EnvironmentObject var container: DIContainer
    @ObservedObject private var languageManager = LanguageManager.shared
    @ObservedObject private var preferences = ReadingPreferences.shared

    @StateObject private var layoutManager = PageLayoutManager()
    @StateObject private var bookmarkManager: BookmarkManager
    @StateObject private var annotationManager: AnnotationManager

    @State private var parseResult: TxtParseResult?
    @State private var currentChapterIndex = 0
    @State private var attributedContent: NSAttributedString = NSAttributedString()
    @State private var isLoading = true
    @State private var errorMessage: String?
    @State private var showTOC = false

    @State private var totalPages = 0
    @State private var currentPageIndex = 0
    @State private var currentChapterPages: [PageSlice] = []
    @State private var currentPageSize: CGSize = .zero
    @State private var chapterPageCounts: [Int] = []
    @State private var totalBookPages = 0

    @State private var savedCharacterOffset = 0
    @State private var pendingRestoreCharacterOffset: Int?
    @State private var contentLoadRequestID = UUID()
    @State private var bookPaginationRequestID = UUID()
    @State private var preloadRequestID = UUID()
    @State private var toolbarRevealToken = 0

    init(book: LibraryBook, engine: any ReaderEngineProtocol) {
        self.book = book
        _bookmarkManager = StateObject(wrappedValue: BookmarkManager(engine: engine, bookId: book.id))
        _annotationManager = StateObject(wrappedValue: AnnotationManager(engine: engine, bookId: book.id))
    }

    private var currentChapterHref: String? {
        "\(currentChapterIndex)"
    }

    private var tocEntries: [TocEntry] {
        guard let parseResult else { return [] }
        return parseResult.chapters.enumerated().map { index, chapter in
            TocEntry(title: chapter.title, href: "\(index)", level: 0, children: [])
        }
    }

    private var theme: ReadingTheme {
        preferences.currentTheme
    }

    private var currentPageCFI: String {
        "txt-chapter-\(currentChapterIndex)-offset-\(savedCharacterOffset)"
    }

    private var currentBookPage: Int {
        let chapterOffset = chapterPageCounts.prefix(currentChapterIndex).reduce(0, +)
        let pageWithinChapter = min(max(currentPageIndex, 0), max(totalPages - 1, 0)) + 1
        return max(1, chapterOffset + pageWithinChapter)
    }

    private var resolvedTotalBookPages: Int {
        max(totalBookPages, chapterPageCounts.reduce(0, +), totalPages)
    }

    private var chapterRemainingPages: Int {
        max(0, totalPages - currentPageIndex - 1)
    }

    private var resolvedReadingProgress: Double {
        guard resolvedTotalBookPages > 0 else { return 0 }
        return min(max(Double(currentBookPage) / Double(resolvedTotalBookPages), 0), 1)
    }

    private var currentBookmarkTitle: String {
        let pageLabel = resolvedTotalBookPages > 0 ? "\(currentBookPage)/\(resolvedTotalBookPages)" : "\(currentPageIndex + 1)"
        return "\(currentChapterTitle) · \(pageLabel)"
    }

    var body: some View {
        ZStack {
            Color(theme.backgroundColor)
                .ignoresSafeArea()

            HStack(spacing: 0) {
                if showTOC {
                    TOCSidebarView(
                        tocEntries: tocEntries,
                        currentChapterHref: currentChapterHref,
                        onSelectChapter: { entry in
                            if let idx = Int(entry.href) {
                                showTOC = false
                                navigateToChapter(index: idx, restoreOffset: 0)
                            }
                        }
                    )
                    .frame(width: 260)
                    .transition(.move(edge: .leading))

                    Divider()
                }

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

                            HStack(spacing: 12) {
                                Button(L("reader.retry")) {
                                    retryCurrentLoad()
                                }
                                .keyboardShortcut(.defaultAction)

                                Button(L("reader.returnToLibrary")) {
                                    returnToLibrary()
                                }
                            }
                        }
                    } else {
                        PaginatedReaderView(
                            attributedString: attributedContent,
                            totalPages: $totalPages,
                            currentPageIndex: $currentPageIndex,
                            layoutManager: layoutManager,
                            preferences: preferences,
                            restoreCharacterOffset: pendingRestoreCharacterOffset,
                            onNextChapter: { navigateChapter(offset: 1) },
                            onPreviousChapter: { navigateChapter(offset: -1) },
                            onCenterTap: { },
                            onPageChange: { offset in
                                savedCharacterOffset = offset
                                pendingRestoreCharacterOffset = nil
                                saveProgress()
                            },
                            onPagesUpdated: handlePagesUpdated
                        )
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .trackPageLayout(manager: layoutManager, tocWidth: showTOC ? 260 : 0)
            .animation(.easeInOut(duration: 0.2), value: showTOC)

            ReaderBottomOverlay(
                currentBookPage: currentBookPage,
                totalBookPages: resolvedTotalBookPages,
                chapterRemainingPages: chapterRemainingPages,
                readingProgress: resolvedReadingProgress,
                onPreviousPage: { navigatePage(offset: -1) },
                onNextPage: { navigatePage(offset: 1) }
            )
        }
        .navigationBarBackButtonHidden(true)
        .readerToolbar(config: ReaderToolbarConfig(
            bookTitle: book.metadata.title,
            chapterTitle: currentChapterTitle,
            canGoPrevious: currentChapterIndex > 0,
            canGoNext: parseResult != nil && currentChapterIndex < (parseResult?.chapters.count ?? 1) - 1,
            showTOC: $showTOC,
            isCurrentPageBookmarked: bookmarkManager.isBookmarked(cfi: currentPageCFI),
            bookmarkManager: bookmarkManager,
            annotationManager: annotationManager,
            searchableText: attributedContent.string,
            onBack: {
                returnToLibrary()
            },
            onPreviousChapter: { navigateChapter(offset: -1) },
            onNextChapter: { navigateChapter(offset: 1) },
            onToggleBookmark: toggleCurrentBookmark,
            onSelectBookmark: jumpToBookmark,
            onSelectAnnotation: jumpToAnnotation,
            onJumpToCharacterOffset: jumpToCharacterOffset
        ))
        .onAppear {
            layoutManager.setLayoutMode(preferences.pageLayoutMode, animated: false)
            bookmarkManager.loadBookmarks()
            annotationManager.loadAnnotations()
            loadContent()
        }
        .onDisappear {
            invalidateAsyncRequests()
            saveProgress()
        }
        .onReceive(NotificationCenter.default.publisher(for: .addBookmark)) { _ in
            toggleCurrentBookmark()
        }
        .onReceive(NotificationCenter.default.publisher(for: .toggleTOC)) { _ in
            withAnimation(.easeInOut(duration: 0.2)) {
                showTOC.toggle()
            }
        }
        .onChange(of: preferences.pageLayoutMode) { _, mode in
            layoutManager.setLayoutMode(mode)
        }
        .onChange(of: preferences.themeHash) { _, _ in
            handleAppearanceChange()
        }
    }

    private var currentChapterTitle: String {
        guard let parseResult, currentChapterIndex < parseResult.chapters.count else { return "" }
        return parseResult.chapters[currentChapterIndex].title
    }

    // MARK: - Loading

    private func loadContent() {
        let requestID = UUID()
        contentLoadRequestID = requestID
        bookPaginationRequestID = UUID()
        preloadRequestID = UUID()

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

            let progress = container.engine.getProgress(bookId: book.id)

            DispatchQueue.main.async {
                guard requestID == contentLoadRequestID else { return }

                guard let loaded else {
                    errorMessage = L("reader.failedLoadTxt")
                    isLoading = false
                    return
                }

                parseResult = loaded
                chapterPageCounts = Array(repeating: 0, count: loaded.chapters.count)
                totalBookPages = 0

                if case .success(let savedProgress) = progress,
                   let restored = parseTxtPosition(cfi: savedProgress.cfiPosition),
                   restored.chapterIndex >= 0,
                   restored.chapterIndex < loaded.chapters.count {
                    currentChapterIndex = restored.chapterIndex
                    savedCharacterOffset = restored.characterOffset
                    pendingRestoreCharacterOffset = restored.characterOffset
                } else {
                    currentChapterIndex = 0
                    savedCharacterOffset = 0
                    pendingRestoreCharacterOffset = 0
                }

                renderCurrentChapter()
                preloadAdjacentChapters()
                isLoading = false
            }
        }
    }

    private func renderCurrentChapter() {
        guard let rendered = renderedChapter(for: currentChapterIndex) else { return }
        attributedContent = rendered
    }

    private func renderedChapter(for index: Int) -> NSAttributedString? {
        guard let parseResult, index >= 0, index < parseResult.chapters.count else { return nil }

        let cacheKey = chapterCacheKey(for: index)
        if let cached = container.chapterCache.get(key: cacheKey) as? NSAttributedString {
            return cached
        }

        let chapter = parseResult.chapters[index]
        let renderer = DomRenderer(
            fontSize: preferences.fontSize,
            lineSpacing: preferences.lineSpacing,
            fontName: preferences.fontName,
            textColor: NSColor(theme.textColor)
        )

        var nodes = chapter.nodes
        let titleNode = DomNode(
            nodeType: .heading(level: 2),
            children: [DomNode(nodeType: .text, text: chapter.title)]
        )
        nodes.insert(titleNode, at: 0)

        let rendered = renderer.render(nodes: nodes)
        container.chapterCache.set(key: cacheKey, value: rendered)
        return rendered
    }

    private func handleAppearanceChange() {
        guard parseResult != nil else { return }
        container.chapterCache.invalidate(bookId: book.id)
        pendingRestoreCharacterOffset = savedCharacterOffset
        renderCurrentChapter()
        preloadAdjacentChapters()
    }

    private func retryCurrentLoad() {
        if parseResult == nil {
            loadContent()
            return
        }
        navigateToChapter(index: currentChapterIndex, restoreOffset: savedCharacterOffset)
    }

    private func returnToLibrary() {
        invalidateAsyncRequests()
        saveProgress()
        appState.exitReader(bookId: book.id)
    }

    // MARK: - Navigation

    private func navigateChapter(offset: Int) {
        let targetIndex = currentChapterIndex + offset
        guard let parseResult, targetIndex >= 0, targetIndex < parseResult.chapters.count else { return }
        navigateToChapter(index: targetIndex, restoreOffset: offset < 0 ? Int.max : 0)
    }

    private func navigateToChapter(index: Int, restoreOffset: Int) {
        guard let parseResult, index >= 0, index < parseResult.chapters.count else { return }
        bookPaginationRequestID = UUID()
        preloadRequestID = UUID()
        currentChapterIndex = index
        pendingRestoreCharacterOffset = restoreOffset
        if restoreOffset != Int.max {
            savedCharacterOffset = max(0, restoreOffset)
        }
        renderCurrentChapter()
        preloadAdjacentChapters()
        currentPageIndex = 0
    }

    private func navigatePage(offset: Int) {
        let step = layoutManager.isDualPage ? 2 : 1
        let newIndex = currentPageIndex + (offset * step)
        if newIndex >= 0 && newIndex < totalPages {
            moveToPage(index: newIndex)
        } else if offset > 0 {
            navigateChapter(offset: 1)
        } else if offset < 0 {
            navigateChapter(offset: -1)
        }
    }

    private func moveToPage(index: Int) {
        guard index >= 0, index < totalPages else { return }
        currentPageIndex = index
        let offset = PaginationEngine.characterOffset(forPageIndex: index, in: currentChapterPages)
        savedCharacterOffset = offset
        pendingRestoreCharacterOffset = nil
        if currentPageIndex <= 1 || chapterRemainingPages <= 1 {
            preloadAdjacentChapters()
        }
        saveProgress()
    }

    private func jumpToCharacterOffset(_ offset: Int) {
        let safeOffset = max(0, offset)
        guard !currentChapterPages.isEmpty else {
            savedCharacterOffset = safeOffset
            pendingRestoreCharacterOffset = safeOffset
            return
        }
        let pageIndex = PaginationEngine.pageIndex(forCharacterOffset: safeOffset, in: currentChapterPages)
        moveToPage(index: pageIndex)
    }

    private func jumpToBookmark(_ bookmark: Bookmark) {
        guard let position = parseTxtPosition(cfi: bookmark.cfiPosition) else { return }
        jumpToReaderPosition(chapterIndex: position.chapterIndex, characterOffset: position.characterOffset)
    }

    private func jumpToAnnotation(_ annotation: Annotation) {
        guard let position = parseTxtPosition(cfi: annotation.cfiStart) else { return }
        jumpToReaderPosition(chapterIndex: position.chapterIndex, characterOffset: position.characterOffset)
    }

    private func jumpToReaderPosition(chapterIndex: Int, characterOffset: Int) {
        if chapterIndex == currentChapterIndex {
            jumpToCharacterOffset(characterOffset)
        } else {
            navigateToChapter(index: chapterIndex, restoreOffset: characterOffset)
        }
    }

    private func toggleCurrentBookmark() {
        bookmarkManager.toggleBookmark(cfi: currentPageCFI, title: currentBookmarkTitle)
    }

    // MARK: - Pagination

    private func handlePagesUpdated(_ pages: [PageSlice], _ pageSize: CGSize) {
        currentChapterPages = pages
        currentPageSize = pageSize
        if let parseResult, chapterPageCounts.count != parseResult.chapters.count {
            chapterPageCounts = Array(repeating: 0, count: parseResult.chapters.count)
        }
        if currentChapterIndex < chapterPageCounts.count {
            chapterPageCounts[currentChapterIndex] = pages.count
        }
        totalBookPages = max(chapterPageCounts.reduce(0, +), pages.count)
        pendingRestoreCharacterOffset = nil
        recalculateBookPagination()
    }

    private func recalculateBookPagination() {
        guard let parseResult, currentPageSize.width > 0, currentPageSize.height > 0 else { return }

        let requestID = UUID()
        bookPaginationRequestID = requestID

        let chapters = parseResult.chapters
        let pageSize = currentPageSize
        let currentIndexSnapshot = currentChapterIndex
        let currentAttributedSnapshot = attributedContent
        let currentPageCount = max(currentChapterPages.count, totalPages)
        let cache = container.chapterCache
        let cacheThemeHash = String(preferences.themeHash)
        let fontSize = preferences.fontSize
        let lineSpacing = preferences.lineSpacing
        let fontName = preferences.fontName
        let textColor = NSColor(theme.textColor)

        DispatchQueue.global(qos: .utility).async {
            var counts = Array(repeating: 0, count: chapters.count)
            for (index, chapter) in chapters.enumerated() {
                guard isBookPaginationRequestCurrent(
                    requestID: requestID,
                    chapterIndex: currentIndexSnapshot,
                    pageSize: pageSize,
                    themeHash: cacheThemeHash
                ) else {
                    return
                }

                let rendered: NSAttributedString
                if index == currentIndexSnapshot {
                    rendered = currentAttributedSnapshot
                    counts[index] = currentPageCount
                    continue
                }

                let cacheKey = ChapterCacheKey(bookId: book.id, chapterPath: "txt-chapter-\(index)", themeHash: cacheThemeHash)
                if let cached = cache.get(key: cacheKey) as? NSAttributedString {
                    rendered = cached
                } else {
                    guard isBookPaginationRequestCurrent(
                        requestID: requestID,
                        chapterIndex: currentIndexSnapshot,
                        pageSize: pageSize,
                        themeHash: cacheThemeHash
                    ) else {
                        return
                    }

                    let renderer = DomRenderer(
                        fontSize: fontSize,
                        lineSpacing: lineSpacing,
                        fontName: fontName,
                        textColor: textColor
                    )
                    var nodes = chapter.nodes
                    let titleNode = DomNode(
                        nodeType: .heading(level: 2),
                        children: [DomNode(nodeType: .text, text: chapter.title)]
                    )
                    nodes.insert(titleNode, at: 0)
                    let newRendered = renderer.render(nodes: nodes)

                    guard isBookPaginationRequestCurrent(
                        requestID: requestID,
                        chapterIndex: currentIndexSnapshot,
                        pageSize: pageSize,
                        themeHash: cacheThemeHash
                    ) else {
                        return
                    }

                    cache.set(key: cacheKey, value: newRendered)
                    rendered = newRendered
                }

                counts[index] = PaginationEngine.paginate(
                    attributedString: rendered,
                    pageSize: pageSize
                ).count
            }

            let total = counts.reduce(0, +)
            DispatchQueue.main.async {
                guard requestID == bookPaginationRequestID,
                      currentIndexSnapshot == currentChapterIndex,
                      currentPageSize == pageSize,
                      cacheThemeHash == String(preferences.themeHash) else { return }
                chapterPageCounts = counts
                totalBookPages = max(total, currentPageCount)
            }
        }
    }

    private func preloadAdjacentChapters() {
        guard let parseResult else { return }
        let targetIndices = preloadCandidateChapterIndices(totalChapterCount: parseResult.chapters.count)
        guard !targetIndices.isEmpty else { return }

        let requestID = UUID()
        preloadRequestID = requestID

        let originChapterIndex = currentChapterIndex
        let fontSize = preferences.fontSize
        let lineSpacing = preferences.lineSpacing
        let fontName = preferences.fontName
        let textColor = NSColor(theme.textColor)
        let cache = container.chapterCache
        let cacheThemeHash = String(preferences.themeHash)

        for idx in targetIndices where idx >= 0 && idx < parseResult.chapters.count {
            let cacheKey = ChapterCacheKey(bookId: book.id, chapterPath: "txt-chapter-\(idx)", themeHash: cacheThemeHash)
            if cache.get(key: cacheKey) != nil { continue }

            let chapter = parseResult.chapters[idx]
            DispatchQueue.global(qos: .background).async {
                guard isPreloadRequestCurrent(
                    requestID: requestID,
                    originChapterIndex: originChapterIndex,
                    themeHash: cacheThemeHash
                ) else {
                    return
                }

                let renderer = DomRenderer(
                    fontSize: fontSize,
                    lineSpacing: lineSpacing,
                    fontName: fontName,
                    textColor: textColor
                )
                var nodes = chapter.nodes
                let titleNode = DomNode(
                    nodeType: .heading(level: 2),
                    children: [DomNode(nodeType: .text, text: chapter.title)]
                )
                nodes.insert(titleNode, at: 0)
                let rendered = renderer.render(nodes: nodes)

                guard isPreloadRequestCurrent(
                    requestID: requestID,
                    originChapterIndex: originChapterIndex,
                    themeHash: cacheThemeHash
                ) else {
                    return
                }

                cache.set(key: cacheKey, value: rendered)
            }
        }
    }

    // MARK: - Progress

    private func saveProgress() {
        guard parseResult != nil else { return }
        let percentage = resolvedTotalBookPages > 0
            ? Double(max(currentBookPage - 1, 0)) / Double(resolvedTotalBookPages) * 100.0
            : 0
        let _ = container.engine.updateProgress(
            bookId: book.id,
            cfi: currentPageCFI,
            percentage: percentage,
            hlcTs: UInt64(Date().timeIntervalSince1970)
        )
    }

    private func invalidateAsyncRequests() {
        contentLoadRequestID = UUID()
        bookPaginationRequestID = UUID()
        preloadRequestID = UUID()
    }

    private func parseTxtPosition(cfi: String) -> (chapterIndex: Int, characterOffset: Int)? {
        guard cfi.hasPrefix("txt-chapter-") else { return nil }
        let parts = cfi.replacingOccurrences(of: "txt-chapter-", with: "").components(separatedBy: "-offset-")
        guard let chapterIndex = Int(parts.first ?? "") else { return nil }
        let offset = parts.count > 1 ? Int(parts[1]) ?? 0 : 0
        return (chapterIndex, offset)
    }

    private func chapterCacheKey(for index: Int) -> ChapterCacheKey {
        ChapterCacheKey(bookId: book.id, chapterPath: "txt-chapter-\(index)", themeHash: String(preferences.themeHash))
    }

    private func preloadCandidateChapterIndices(totalChapterCount: Int) -> [Int] {
        guard totalChapterCount > 0 else { return [] }

        if totalPages <= 2 {
            return [currentChapterIndex - 1, currentChapterIndex + 1]
                .filter { $0 >= 0 && $0 < totalChapterCount }
        }

        var indices: [Int] = []
        if currentPageIndex <= 1, currentChapterIndex > 0 {
            indices.append(currentChapterIndex - 1)
        }
        if chapterRemainingPages <= 1, currentChapterIndex + 1 < totalChapterCount {
            indices.append(currentChapterIndex + 1)
        }
        if indices.isEmpty, currentChapterIndex + 1 < totalChapterCount {
            indices.append(currentChapterIndex + 1)
        }
        return indices
    }

    private func isBookPaginationRequestCurrent(
        requestID: UUID,
        chapterIndex: Int,
        pageSize: CGSize,
        themeHash: String
    ) -> Bool {
        let check = {
            requestID == bookPaginationRequestID &&
            chapterIndex == currentChapterIndex &&
            pageSize == currentPageSize &&
            themeHash == String(preferences.themeHash)
        }
        if Thread.isMainThread {
            return check()
        }
        return DispatchQueue.main.sync(execute: check)
    }

    private func isPreloadRequestCurrent(
        requestID: UUID,
        originChapterIndex: Int,
        themeHash: String
    ) -> Bool {
        let check = {
            requestID == preloadRequestID &&
            originChapterIndex == currentChapterIndex &&
            themeHash == String(preferences.themeHash) &&
            !isLoading
        }
        if Thread.isMainThread {
            return check()
        }
        return DispatchQueue.main.sync(execute: check)
    }
}
