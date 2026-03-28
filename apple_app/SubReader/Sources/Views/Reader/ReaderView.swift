// ReaderView — Main reading view with DOM rendering and chapter navigation.
//
// Uses PaginatedReaderView for Apple Books-style paginated reading.
// Implements chapter preloading and NSAttributedString caching.

import SwiftUI
import Combine
import ReaderModels
import ReaderBridge

struct ReaderView: View {
    let bookId: String

    @EnvironmentObject var appState: AppState
    @EnvironmentObject var container: DIContainer
    @ObservedObject private var languageManager = LanguageManager.shared
    @ObservedObject private var preferences = ReadingPreferences.shared

    @StateObject private var layoutManager = PageLayoutManager()
    @StateObject private var bookmarkManager: BookmarkManager
    @StateObject private var annotationManager: AnnotationManager

    @State private var metadata: BookMetadata?
    @State private var currentChapterIndex: Int = 0
    @State private var chapterNodes: [DomNode] = []
    @State private var attributedContent: NSAttributedString = NSAttributedString()
    @State private var progress: ReadingProgress?
    @State private var isLoading = true
    @State private var errorMessage: String?

    @State private var spinePaths: [String] = []
    @State private var tocEntries: [TocEntry] = []
    @State private var flatTocEntries: [TocEntry] = []
    @State private var showTOC = false

    @State private var totalPages = 0
    @State private var currentPageIndex = 0
    @State private var currentChapterPages: [PageSlice] = []
    @State private var currentPageSize: CGSize = .zero
    @State private var chapterPageCounts: [Int] = []
    @State private var totalBookPages = 0

    @State private var savedCharacterOffset = 0
    @State private var pendingRestoreCharacterOffset: Int?
    @State private var progressSaveTask: DispatchWorkItem?
    @State private var progressWriteRequestID = UUID()
    @State private var bookLoadRequestID = UUID()
    @State private var chapterLoadRequestID = UUID()
    @State private var bookPaginationRequestID = UUID()
    @State private var preloadRequestID = UUID()
    @State private var toolbarRevealToken = 0

    init(bookId: String, engine: any ReaderEngineProtocol) {
        self.bookId = bookId
        _bookmarkManager = StateObject(wrappedValue: BookmarkManager(engine: engine, bookId: bookId))
        _annotationManager = StateObject(wrappedValue: AnnotationManager(engine: engine, bookId: bookId))
    }

    private var currentChapterHref: String? {
        guard currentChapterIndex < spinePaths.count else { return nil }
        return spinePaths[currentChapterIndex]
    }

    private var currentChapterPath: String? {
        guard currentChapterIndex >= 0, currentChapterIndex < spinePaths.count else { return nil }
        return spinePaths[currentChapterIndex]
    }

    private var theme: ReadingTheme {
        preferences.currentTheme
    }

    private var currentPageCFI: String {
        "page-\(currentChapterIndex)-\(savedCharacterOffset)"
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
        guard resolvedTotalBookPages > 0 else {
            return min(max((progress?.percentage ?? 0) / 100.0, 0), 1)
        }
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
                        onSelectChapter: handleTocSelection
                    )
                    .frame(width: 260)
                    .transition(.move(edge: .leading))

                    Divider()
                }

                ZStack {
                    if isLoading {
                        ProgressView(L("reader.loadingChapter"))
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
                                debounceSaveProgress()
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
            bookTitle: metadata?.title ?? L("reader.reading"),
            chapterTitle: currentChapterTitle,
            canGoPrevious: currentChapterIndex > 0,
            canGoNext: metadata != nil && currentChapterIndex < spinePaths.count - 1,
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
            loadBook()
        }
        .onDisappear {
            saveProgressAndClose()
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
        if currentChapterIndex < spinePaths.count {
            let currentPath = spinePaths[currentChapterIndex]
            if let entry = flatTocEntries.first(where: { href in
                let basePath = currentPath.components(separatedBy: "#").first ?? currentPath
                let tocBase = href.href.components(separatedBy: "#").first ?? href.href
                return basePath.hasSuffix(tocBase) || tocBase.hasSuffix(basePath)
            }) {
                return entry.title
            }
        }
        return L("reader.chapter", currentChapterIndex + 1)
    }

    // MARK: - Data Loading

    private func loadBook() {
        let requestID = UUID()
        bookLoadRequestID = requestID
        chapterLoadRequestID = UUID()
        bookPaginationRequestID = UUID()
        preloadRequestID = UUID()

        isLoading = true
        errorMessage = nil

        guard let book = appState.libraryBooks.first(where: { $0.id == bookId }) else {
            errorMessage = L("reader.bookNotFound")
            isLoading = false
            return
        }

        DispatchQueue.global(qos: .userInitiated).async {
            guard appState.reopenEpubForReading(book) else {
                DispatchQueue.main.async {
                    guard requestID == bookLoadRequestID else { return }
                    errorMessage = L("reader.failedOpenEpub")
                    isLoading = false
                }
                return
            }

            let metaResult = appState.engine.getMetadata()
            guard case .success(let meta) = metaResult else {
                DispatchQueue.main.async {
                    guard requestID == bookLoadRequestID else { return }
                    errorMessage = L("reader.failedLoadMetadata")
                    isLoading = false
                }
                return
            }

            let spine = (try? appState.engine.getSpine().get()) ?? []
            let toc = (try? appState.engine.getToc().get()) ?? []
            let flatToc = flattenToc(toc)
            let prog = try? appState.engine.getProgress(bookId: bookId).get()

            var restoredChapterIndex = 0
            var restoredCharacterOffset = 0
            if let prog,
               let restored = parseReaderPosition(cfi: prog.cfiPosition) {
                restoredChapterIndex = min(max(restored.chapterIndex, 0), max(0, spine.count - 1))
                restoredCharacterOffset = max(0, restored.characterOffset)
            }

            let chapterPath = restoredChapterIndex < spine.count ? spine[restoredChapterIndex] : (spine.first ?? "")
            let contentResult: Result<[DomNode], ReaderError>
            if !chapterPath.isEmpty {
                contentResult = appState.engine.getChapterContent(path: chapterPath)
            } else {
                contentResult = .failure(.notFound)
            }

            DispatchQueue.main.async {
                guard requestID == bookLoadRequestID else { return }

                chapterLoadRequestID = requestID
                metadata = meta
                spinePaths = spine
                tocEntries = toc
                flatTocEntries = flatToc
                progress = prog
                chapterPageCounts = Array(repeating: 0, count: spine.count)
                totalBookPages = 0
                currentChapterIndex = restoredChapterIndex
                savedCharacterOffset = restoredCharacterOffset
                pendingRestoreCharacterOffset = restoredCharacterOffset

                switch contentResult {
                case .success(let nodes):
                    chapterNodes = nodes
                    renderContent()
                    currentPageIndex = 0
                    preloadAdjacentChapters()
                case .failure:
                    if spine.isEmpty {
                        errorMessage = L("reader.noChapters")
                    } else {
                        errorMessage = L("reader.failedLoadChapter")
                    }
                }
                isLoading = false
            }
        }
    }

    private func renderContent() {
        let renderer = DomRenderer(
            fontSize: preferences.fontSize,
            lineSpacing: preferences.lineSpacing,
            fontName: preferences.fontName,
            textColor: NSColor(theme.textColor)
        )
        let rendered = renderer.render(nodes: chapterNodes)
        attributedContent = rendered
        if let currentChapterPath {
            container.chapterCache.set(key: chapterCacheKey(for: currentChapterPath), value: rendered)
        }
    }

    private func handleAppearanceChange() {
        guard !chapterNodes.isEmpty else { return }
        container.chapterCache.invalidate(bookId: bookId)
        pendingRestoreCharacterOffset = savedCharacterOffset
        renderContent()
        preloadAdjacentChapters()
    }

    private func retryCurrentLoad() {
        if metadata == nil || spinePaths.isEmpty {
            loadBook()
            return
        }
        navigateToChapter(index: currentChapterIndex, restoreOffset: savedCharacterOffset)
    }

    private func returnToLibrary() {
        saveProgressAndClose()
        appState.exitReader(bookId: bookId)
    }

    // MARK: - Navigation

    private func navigateChapter(offset: Int) {
        let newIndex = currentChapterIndex + offset
        guard newIndex >= 0, newIndex < spinePaths.count else { return }
        navigateToChapter(index: newIndex, restoreOffset: offset < 0 ? Int.max : 0)
    }

    private func navigateToChapter(index: Int, restoreOffset: Int) {
        guard index >= 0, index < spinePaths.count else { return }

        let requestID = UUID()
        chapterLoadRequestID = requestID
        bookPaginationRequestID = UUID()
        preloadRequestID = UUID()

        currentChapterIndex = index
        isLoading = true
        errorMessage = nil
        pendingRestoreCharacterOffset = restoreOffset
        if restoreOffset != Int.max {
            savedCharacterOffset = max(0, restoreOffset)
        }

        let chapterPath = spinePaths[index]
        DispatchQueue.global(qos: .userInitiated).async {
            let result = appState.engine.getChapterContent(path: chapterPath)
            DispatchQueue.main.async {
                guard requestID == chapterLoadRequestID else { return }

                switch result {
                case .success(let nodes):
                    chapterNodes = nodes
                    renderContent()
                    currentPageIndex = 0
                    preloadAdjacentChapters()
                case .failure:
                    errorMessage = L("reader.failedLoadChapterShort")
                }
                isLoading = false
            }
        }
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
        debounceSaveProgress()
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
        guard let position = parseReaderPosition(cfi: bookmark.cfiPosition) else { return }
        jumpToReaderPosition(chapterIndex: position.chapterIndex, characterOffset: position.characterOffset)
    }

    private func jumpToAnnotation(_ annotation: Annotation) {
        guard let position = parseReaderPosition(cfi: annotation.cfiStart) else { return }
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
        if chapterPageCounts.count != spinePaths.count {
            chapterPageCounts = Array(repeating: 0, count: spinePaths.count)
        }
        if currentChapterIndex < chapterPageCounts.count {
            chapterPageCounts[currentChapterIndex] = pages.count
        }
        totalBookPages = max(chapterPageCounts.reduce(0, +), pages.count)
        pendingRestoreCharacterOffset = nil
        recalculateBookPagination()
    }

    private func recalculateBookPagination() {
        guard currentPageSize.width > 0, currentPageSize.height > 0, !spinePaths.isEmpty else { return }

        let requestID = UUID()
        bookPaginationRequestID = requestID

        let spineSnapshot = spinePaths
        let currentIndexSnapshot = currentChapterIndex
        let currentAttributedSnapshot = attributedContent
        let currentPageCount = max(currentChapterPages.count, totalPages)
        let pageSize = currentPageSize
        let cache = container.chapterCache
        let engine = appState.engine
        let cacheThemeHash = String(preferences.themeHash)
        let fontSize = preferences.fontSize
        let lineSpacing = preferences.lineSpacing
        let fontName = preferences.fontName
        let textColor = NSColor(theme.textColor)

        DispatchQueue.global(qos: .utility).async {
            var counts = Array(repeating: 0, count: spineSnapshot.count)
            for (index, path) in spineSnapshot.enumerated() {
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

                let cacheKey = ChapterCacheKey(bookId: bookId, chapterPath: path, themeHash: cacheThemeHash)
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

                    let result = engine.getChapterContent(path: path)

                    guard isBookPaginationRequestCurrent(
                        requestID: requestID,
                        chapterIndex: currentIndexSnapshot,
                        pageSize: pageSize,
                        themeHash: cacheThemeHash
                    ) else {
                        return
                    }

                    guard case .success(let nodes) = result else { continue }
                    let renderer = DomRenderer(
                        fontSize: fontSize,
                        lineSpacing: lineSpacing,
                        fontName: fontName,
                        textColor: textColor
                    )
                    let newRendered = renderer.render(nodes: nodes)
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
        let indices = preloadCandidateChapterIndices()
        guard !indices.isEmpty else { return }

        let requestID = UUID()
        preloadRequestID = requestID

        let originChapterIndex = currentChapterIndex
        let fontSize = preferences.fontSize
        let lineSpacing = preferences.lineSpacing
        let fontName = preferences.fontName
        let textColor = NSColor(theme.textColor)
        let cache = container.chapterCache
        let cacheThemeHash = String(preferences.themeHash)

        for idx in indices where idx >= 0 && idx < spinePaths.count {
            let path = spinePaths[idx]
            let cacheKey = ChapterCacheKey(bookId: bookId, chapterPath: path, themeHash: cacheThemeHash)
            if cache.get(key: cacheKey) != nil { continue }

            DispatchQueue.global(qos: .background).async {
                guard isPreloadRequestCurrent(
                    requestID: requestID,
                    originChapterIndex: originChapterIndex,
                    themeHash: cacheThemeHash
                ) else {
                    return
                }

                let result = appState.engine.getChapterContent(path: path)

                guard isPreloadRequestCurrent(
                    requestID: requestID,
                    originChapterIndex: originChapterIndex,
                    themeHash: cacheThemeHash
                ) else {
                    return
                }

                guard case .success(let nodes) = result else { return }
                let renderer = DomRenderer(
                    fontSize: fontSize,
                    lineSpacing: lineSpacing,
                    fontName: fontName,
                    textColor: textColor
                )
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

    // MARK: - Progress Tracking

    private func debounceSaveProgress() {
        progressSaveTask?.cancel()
        let requestID = UUID()
        progressWriteRequestID = requestID
        let task = DispatchWorkItem {
            let percentage = resolvedTotalBookPages > 0
                ? Double(max(currentBookPage - 1, 0)) / Double(resolvedTotalBookPages) * 100.0
                : 0
            let cfi = currentPageCFI
            let _ = appState.engine.updateProgress(
                bookId: bookId,
                cfi: cfi,
                percentage: percentage,
                hlcTs: UInt64(Date().timeIntervalSince1970)
            )
            DispatchQueue.main.async {
                guard requestID == progressWriteRequestID else { return }
                progress = ReadingProgress(
                    bookId: bookId,
                    cfiPosition: cfi,
                    percentage: percentage,
                    hlcTimestamp: UInt64(Date().timeIntervalSince1970)
                )
            }
        }
        progressSaveTask = task
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 0.5, execute: task)
    }

    private func saveProgressAndClose() {
        invalidateAsyncRequests()
        progressSaveTask?.cancel()
        let percentage = resolvedTotalBookPages > 0
            ? Double(max(currentBookPage - 1, 0)) / Double(resolvedTotalBookPages) * 100.0
            : (progress?.percentage ?? 0)
        let cfi = currentPageCFI
        let _ = appState.engine.updateProgress(
            bookId: bookId,
            cfi: cfi,
            percentage: percentage,
            hlcTs: UInt64(Date().timeIntervalSince1970)
        )
        let _ = appState.engine.closeBook()
    }

    private func invalidateAsyncRequests() {
        progressWriteRequestID = UUID()
        bookLoadRequestID = UUID()
        chapterLoadRequestID = UUID()
        bookPaginationRequestID = UUID()
        preloadRequestID = UUID()
    }

    // MARK: - Helpers

    private func chapterCacheKey(for path: String) -> ChapterCacheKey {
        ChapterCacheKey(bookId: bookId, chapterPath: path, themeHash: String(preferences.themeHash))
    }

    private func preloadCandidateChapterIndices() -> [Int] {
        guard !spinePaths.isEmpty else { return [] }

        if totalPages <= 2 {
            return [currentChapterIndex - 1, currentChapterIndex + 1]
                .filter { $0 >= 0 && $0 < spinePaths.count }
        }

        var indices: [Int] = []
        if currentPageIndex <= 1, currentChapterIndex > 0 {
            indices.append(currentChapterIndex - 1)
        }
        if chapterRemainingPages <= 1, currentChapterIndex + 1 < spinePaths.count {
            indices.append(currentChapterIndex + 1)
        }
        if indices.isEmpty, currentChapterIndex + 1 < spinePaths.count {
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

    private func flattenToc(_ entries: [TocEntry]) -> [TocEntry] {
        var result: [TocEntry] = []
        for entry in entries {
            result.append(entry)
            result.append(contentsOf: flattenToc(entry.children))
        }
        return result
    }

    private func parseReaderPosition(cfi: String) -> (chapterIndex: Int, characterOffset: Int)? {
        guard cfi.hasPrefix("page-") else { return nil }
        let parts = cfi.replacingOccurrences(of: "page-", with: "").split(separator: "-", maxSplits: 1)
        guard let chapterPart = parts.first,
              let chapterIndex = Int(chapterPart) else { return nil }
        let offset = parts.count > 1 ? Int(parts[1]) ?? 0 : 0
        return (chapterIndex, offset)
    }

    // MARK: - TOC Selection

    private func handleTocSelection(_ entry: TocEntry) {
        let result = container.engine.resolveTocHref(href: entry.href)
        if case .success(let spineIndex) = result {
            showTOC = false
            navigateToChapter(index: spineIndex, restoreOffset: 0)
        }
    }
}
