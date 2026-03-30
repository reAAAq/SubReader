// ReaderToolbarOverlay — Apple Books-style reader chrome.
//
// Two parts:
// 1. ReaderToolbarModifier — native .toolbar items that sit in the system titlebar
//    row alongside the traffic-light buttons (fully integrated, not a separate bar).
// 2. ReaderBottomOverlay — bottom status bar + ghost page arrows (always visible).

import SwiftUI
import AppKit
import ReaderModels

private let defaultWindowTitle =
    (Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String)
    ?? (Bundle.main.object(forInfoDictionaryKey: "CFBundleName") as? String)
    ?? "SubReader"

private struct ReaderWindowTitleVisibilityView: NSViewRepresentable {
    let isHidden: Bool

    func makeNSView(context: Context) -> ReaderWindowTitleVisibilityNSView {
        let view = ReaderWindowTitleVisibilityNSView()
        view.isTitleHidden = isHidden
        return view
    }

    func updateNSView(_ nsView: ReaderWindowTitleVisibilityNSView, context: Context) {
        nsView.isTitleHidden = isHidden
    }

    static func dismantleNSView(_ nsView: ReaderWindowTitleVisibilityNSView, coordinator: ()) {
        nsView.isTitleHidden = false
    }
}

private final class ReaderWindowTitleVisibilityNSView: NSView {
    var isTitleHidden = false {
        didSet { applyWindowTitleVisibility() }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        applyWindowTitleVisibility()
    }

    override func viewWillMove(toWindow newWindow: NSWindow?) {
        if let currentWindow = window, newWindow == nil {
            currentWindow.titleVisibility = .visible
            currentWindow.title = defaultWindowTitle
        }
        super.viewWillMove(toWindow: newWindow)
    }

    private func applyWindowTitleVisibility() {
        guard let window else { return }
        window.titleVisibility = isTitleHidden ? .hidden : .visible
        window.title = isTitleHidden ? "" : defaultWindowTitle
    }
}

private extension View {
    @ViewBuilder
    func readerWindowTitleHidden() -> some View {
        if #available(macOS 15.0, *) {
            self.toolbar(removing: .title)
        } else {
            self.background(ReaderWindowTitleVisibilityView(isHidden: true))
        }
    }
}

// MARK: - Collection Section (shared)

private enum ReaderCollectionSection: String, CaseIterable, Identifiable {
    case bookmarks
    case annotations
    var id: String { rawValue }
}

// MARK: - Search Result (shared)

private struct ChapterSearchResult: Identifiable, Hashable {
    let range: NSRange
    let preview: String
    var id: String { "\(range.location)-\(range.length)" }
}

// MARK: - Toolbar Configuration (passed to modifier)

/// All the state & callbacks the native toolbar needs.
struct ReaderToolbarConfig {
    let bookTitle: String
    let chapterTitle: String
    let canGoPrevious: Bool
    let canGoNext: Bool
    var showTOC: Binding<Bool>

    var isCurrentPageBookmarked: Bool = false
    var bookmarkManager: BookmarkManager?
    var annotationManager: AnnotationManager?
    var searchableText: String = ""

    var onBack: (() -> Void)?
    var onPreviousChapter: (() -> Void)?
    var onNextChapter: (() -> Void)?
    var onToggleBookmark: (() -> Void)?
    var onSelectBookmark: ((Bookmark) -> Void)?
    var onSelectAnnotation: ((Annotation) -> Void)?
    var onJumpToCharacterOffset: ((Int) -> Void)?
}

// MARK: - Native Toolbar Modifier

/// Adds reader toolbar items into the system titlebar (same row as traffic lights).
struct ReaderToolbarModifier: ViewModifier {
    let config: ReaderToolbarConfig

    @ObservedObject private var preferences = ReadingPreferences.shared
    @State private var showDisplayPopover = false
    @State private var showSearchPopover = false
    @State private var showCollectionsPopover = false
    @State private var collectionSection: ReaderCollectionSection = .bookmarks
    @State private var searchQuery = ""
    @State private var searchResults: [ChapterSearchResult] = []
    @State private var selectedSearchResultID: ChapterSearchResult.ID?
    @State private var searchRefreshTask: DispatchWorkItem?
    @State private var searchRequestID = UUID()
    @State private var areSearchResultsTruncated = false

    private var theme: ReadingTheme { preferences.currentTheme }
    private var maxSearchResults: Int { 50 }

    func body(content: Content) -> some View {
        content
            .toolbar {
                ToolbarItemGroup(placement: .navigation) {
                    Button {
                        withAnimation(.easeInOut(duration: 0.2)) {
                            config.showTOC.wrappedValue.toggle()
                        }
                    } label: {
                        Image(systemName: "list.bullet")
                    }
                    .help(L("sidebar.toc"))

                    Menu {
                        Button(L("reader.layoutAutomatic")) {
                            preferences.pageLayoutMode = .automatic
                        }
                        Button(L("reader.layoutSingle")) {
                            preferences.pageLayoutMode = .single
                        }
                        Button(L("reader.layoutDual")) {
                            preferences.pageLayoutMode = .dual
                        }
                    } label: {
                        Image(systemName: layoutIconName)
                    }
                    .help(L("reader.layout"))

                    Button {
                        toggleCollectionsPopover(for: .bookmarks)
                    } label: {
                        Image(systemName: "doc.plaintext")
                    }
                    .help(L("reader.bookmarksAndAnnotations"))
                    .popover(isPresented: $showCollectionsPopover, arrowEdge: .bottom) {
                        collectionsPopoverContent
                    }
                }

                ToolbarItem(placement: .principal) {
                    Text(config.bookTitle)
                        .font(.system(size: 13, weight: .medium))
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .frame(maxWidth: 400)
                        .help(config.chapterTitle)
                }

                ToolbarItemGroup(placement: .primaryAction) {
                    Button {
                        toggleDisplayPopover()
                    } label: {
                        Text(L("reader.fontSize"))
                            .font(.system(size: 13, weight: .regular))
                    }
                    .popover(isPresented: $showDisplayPopover, arrowEdge: .bottom) {
                        displayPopoverContent
                    }

                    Button {
                        toggleSearchPopover()
                    } label: {
                        Image(systemName: "magnifyingglass")
                    }
                    .help(L("reader.search"))
                    .popover(isPresented: $showSearchPopover, arrowEdge: .bottom) {
                        searchPopoverContent
                    }

                    Button {
                        config.onToggleBookmark?()
                    } label: {
                        Image(systemName: config.isCurrentPageBookmarked ? "bookmark.fill" : "bookmark")
                    }
                    .help(L("commands.bookmarkCurrentPage"))
                }
            }
            .readerWindowTitleHidden()
            .toolbarBackground(.hidden, for: .windowToolbar)
            .onKeyPress(.escape) {
                if dismissVisiblePopover() {
                    return .handled
                }
                config.onBack?()
                return .handled
            }
            .onChange(of: config.searchableText) { _, _ in
                scheduleSearchRefresh(selectFirst: true)
            }
            .onChange(of: searchQuery) { _, _ in
                scheduleSearchRefresh(selectFirst: true)
            }
            .onChange(of: showSearchPopover) { _, isPresented in
                if isPresented {
                    scheduleSearchRefresh(selectFirst: true)
                } else {
                    clearSearchState(clearQuery: true)
                }
            }
            .onReceive(NotificationCenter.default.publisher(for: .toggleReaderSearch)) { _ in
                toggleSearchPopover()
            }
            .onReceive(NotificationCenter.default.publisher(for: .toggleReaderDisplay)) { _ in
                toggleDisplayPopover()
            }
    }

    // MARK: - Layout Icons

    /// Icon for the page layout menu in the reader titlebar.
    private var layoutIconName: String {
        switch preferences.pageLayoutMode {
        case .automatic: return "rectangle.3.group"
        case .single: return "rectangle.portrait"
        case .dual: return "rectangle.split.2x1"
        }
    }

    // MARK: - Display Popover

    private var displayPopoverContent: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(L("reader.display"))
                .font(.system(size: 13, weight: .semibold))

            VStack(alignment: .leading, spacing: 8) {
                Text(L("settings.readingTheme"))
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)

                HStack(spacing: 14) {
                    themeCircle(type: .light, color: .white, borderColor: .gray)
                    themeCircle(type: .sepia, color: Color(red: 0.96, green: 0.93, blue: 0.87), borderColor: Color(red: 0.8, green: 0.7, blue: 0.5))
                    themeCircle(type: .dark, color: Color(nsColor: .init(white: 0.12, alpha: 1)), borderColor: .gray)
                }
            }

            Divider()

            VStack(alignment: .leading, spacing: 8) {
                Text(L("settings.fontSize"))
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)

                HStack(spacing: 14) {
                    Button {
                        if preferences.fontSize > 12 { preferences.fontSize -= 1 }
                    } label: {
                        Text("A")
                            .font(.system(size: 12, weight: .medium))
                            .frame(width: 28, height: 28)
                    }
                    .buttonStyle(.borderless)

                    Text("\(Int(preferences.fontSize))")
                        .font(.system(size: 13, weight: .semibold))
                        .frame(minWidth: 28)

                    Button {
                        if preferences.fontSize < 32 { preferences.fontSize += 1 }
                    } label: {
                        Text("A")
                            .font(.system(size: 16, weight: .medium))
                            .frame(width: 28, height: 28)
                    }
                    .buttonStyle(.borderless)
                }
            }

            VStack(alignment: .leading, spacing: 8) {
                Text(L("settings.lineSpacing"))
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)

                HStack(spacing: 14) {
                    Button {
                        preferences.lineSpacing = max(1.0, (preferences.lineSpacing - 0.1).rounded(toPlaces: 1))
                    } label: {
                        Text("−")
                            .font(.system(size: 14, weight: .medium))
                            .frame(width: 28, height: 28)
                    }
                    .buttonStyle(.borderless)

                    Text(String(format: "%.1f", preferences.lineSpacing))
                        .font(.system(size: 13, weight: .semibold))
                        .frame(minWidth: 28)

                    Button {
                        preferences.lineSpacing = min(2.4, (preferences.lineSpacing + 0.1).rounded(toPlaces: 1))
                    } label: {
                        Text("+")
                            .font(.system(size: 14, weight: .medium))
                            .frame(width: 28, height: 28)
                    }
                    .buttonStyle(.borderless)
                }
            }

            Divider()

            VStack(alignment: .leading, spacing: 8) {
                Text(L("reader.layout"))
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)

                Picker("", selection: Binding(
                    get: { preferences.pageLayoutMode },
                    set: { preferences.pageLayoutMode = $0 }
                )) {
                    Text(L("reader.layoutAutomatic")).tag(ReaderPageLayoutMode.automatic)
                    Text(L("reader.layoutSingle")).tag(ReaderPageLayoutMode.single)
                    Text(L("reader.layoutDual")).tag(ReaderPageLayoutMode.dual)
                }
                .pickerStyle(.segmented)
            }
        }
        .padding(16)
        .frame(width: 280)
    }

    // MARK: - Collections Popover

    private var collectionsPopoverContent: some View {
        VStack(spacing: 12) {
            Picker("", selection: $collectionSection) {
                Text(L("bookmarks.title")).tag(ReaderCollectionSection.bookmarks)
                Text(L("annotations.title")).tag(ReaderCollectionSection.annotations)
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, 16)
            .padding(.top, 16)

            Group {
                if collectionSection == .bookmarks {
                    if let bookmarkManager = config.bookmarkManager {
                        BookmarkListView(manager: bookmarkManager) { bookmark in
                            showCollectionsPopover = false
                            config.onSelectBookmark?(bookmark)
                        }
                    } else {
                        emptyCollectionsView(title: L("bookmarks.noBookmarks"))
                    }
                } else {
                    if let annotationManager = config.annotationManager {
                        AnnotationListView(manager: annotationManager) { annotation in
                            showCollectionsPopover = false
                            config.onSelectAnnotation?(annotation)
                        }
                    } else {
                        emptyCollectionsView(title: L("annotations.noAnnotations"))
                    }
                }
            }
        }
        .frame(width: 320, height: 360)
    }

    private func emptyCollectionsView(title: String) -> some View {
        VStack(spacing: 8) {
            Spacer()
            Text(title)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(.secondary)
            Spacer()
        }
    }

    // MARK: - Search Popover

    private var searchPopoverContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L("reader.search"))
                .font(.system(size: 13, weight: .semibold))

            TextField(L("reader.searchPlaceholder"), text: $searchQuery)
                .textFieldStyle(.roundedBorder)
                .onSubmit {
                    jumpToSearchResult(at: 0)
                }

            HStack {
                Text(searchSummaryText)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)

                Spacer()

                Button {
                    selectPreviousSearchResult()
                } label: {
                    Image(systemName: "chevron.up")
                }
                .buttonStyle(.borderless)
                .disabled(searchResults.isEmpty)

                Button {
                    selectNextSearchResult()
                } label: {
                    Image(systemName: "chevron.down")
                }
                .buttonStyle(.borderless)
                .disabled(searchResults.isEmpty)
            }

            if searchResults.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text(searchQuery.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                         ? L("reader.searchPlaceholder")
                         : L("reader.noSearchResults"))
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, minHeight: 120, alignment: .topLeading)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 6) {
                        ForEach(Array(searchResults.enumerated()), id: \.element.id) { index, result in
                            Button {
                                jumpToSearchResult(at: index)
                            } label: {
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(result.preview)
                                        .font(.system(size: 12))
                                        .foregroundStyle(.primary)
                                        .multilineTextAlignment(.leading)
                                        .frame(maxWidth: .infinity, alignment: .leading)

                                    Text("#\(index + 1)")
                                        .font(.system(size: 10, weight: .medium))
                                        .foregroundStyle(.secondary)
                                }
                                .padding(.horizontal, 10)
                                .padding(.vertical, 6)
                                .background(
                                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                                        .fill(selectedSearchResultID == result.id ? Color.accentColor.opacity(0.1) : Color.clear)
                                )
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                .frame(maxHeight: 220)
            }
        }
        .padding(16)
        .frame(width: 320)
    }

    private var searchSummaryText: String {
        if searchResults.isEmpty {
            return searchQuery.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                ? L("reader.searchPlaceholder")
                : L("reader.noSearchResults")
        }
        let totalLabel = areSearchResultsTruncated ? "\(searchResults.count)+" : "\(searchResults.count)"
        if let selectedSearchResultID,
           let idx = searchResults.firstIndex(where: { $0.id == selectedSearchResultID }) {
            return "\(idx + 1) / \(totalLabel)"
        }
        return totalLabel
    }

    // MARK: - Search Logic

    private func toggleDisplayPopover() {
        let shouldOpen = !showDisplayPopover
        dismissAllPopovers(except: .display)
        showDisplayPopover = shouldOpen
    }

    private func toggleSearchPopover() {
        let shouldOpen = !showSearchPopover
        dismissAllPopovers(except: .search)
        showSearchPopover = shouldOpen
    }

    private func toggleCollectionsPopover(for section: ReaderCollectionSection) {
        let shouldOpen = !showCollectionsPopover || collectionSection != section
        dismissAllPopovers(except: .collections)
        collectionSection = section
        showCollectionsPopover = shouldOpen
    }

    private enum ActivePopover {
        case display
        case search
        case collections
    }

    private func dismissAllPopovers(except activePopover: ActivePopover? = nil) {
        if activePopover != .display {
            showDisplayPopover = false
        }
        if activePopover != .search {
            showSearchPopover = false
        }
        if activePopover != .collections {
            showCollectionsPopover = false
        }
    }

    private func dismissVisiblePopover() -> Bool {
        let hasVisiblePopover = showDisplayPopover || showSearchPopover || showCollectionsPopover
        guard hasVisiblePopover else { return false }
        dismissAllPopovers()
        return true
    }

    private func clearSearchState(clearQuery: Bool = false) {
        searchRefreshTask?.cancel()
        searchRefreshTask = nil
        searchRequestID = UUID()
        searchResults = []
        selectedSearchResultID = nil
        areSearchResultsTruncated = false
        if clearQuery {
            searchQuery = ""
        }
    }

    private func scheduleSearchRefresh(selectFirst: Bool = false) {
        searchRefreshTask?.cancel()

        guard showSearchPopover else {
            clearSearchState()
            return
        }

        let trimmed = searchQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        let searchableText = config.searchableText
        guard !trimmed.isEmpty, !searchableText.isEmpty else {
            clearSearchState()
            return
        }

        let requestID = UUID()
        searchRequestID = requestID
        let task = DispatchWorkItem {
            let (matches, isTruncated) = buildSearchResults(for: trimmed, in: searchableText)
            DispatchQueue.main.async {
                guard requestID == searchRequestID,
                      showSearchPopover,
                      trimmed == searchQuery.trimmingCharacters(in: .whitespacesAndNewlines),
                      searchableText == config.searchableText else { return }
                applySearchResults(matches, isTruncated: isTruncated, selectFirst: selectFirst)
            }
        }

        searchRefreshTask = task
        DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + 0.2, execute: task)
    }

    private func refreshSearchResults(selectFirst: Bool = false) {
        let trimmed = searchQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        let searchableText = config.searchableText
        guard showSearchPopover, !trimmed.isEmpty, !searchableText.isEmpty else {
            clearSearchState()
            return
        }

        searchRefreshTask?.cancel()
        let (matches, isTruncated) = buildSearchResults(for: trimmed, in: searchableText)
        applySearchResults(matches, isTruncated: isTruncated, selectFirst: selectFirst)
    }

    private func buildSearchResults(for query: String, in text: String) -> ([ChapterSearchResult], Bool) {
        let source = text as NSString
        var matches: [ChapterSearchResult] = []
        var searchRange = NSRange(location: 0, length: source.length)
        var isTruncated = false

        while searchRange.location < source.length {
            if matches.count >= maxSearchResults {
                isTruncated = true
                break
            }

            let found = source.range(of: query, options: [.caseInsensitive, .diacriticInsensitive], range: searchRange)
            if found.location == NSNotFound || found.length == 0 { break }
            matches.append(ChapterSearchResult(range: found, preview: snippet(for: found, in: text)))
            let next = found.location + found.length
            searchRange = NSRange(location: next, length: max(0, source.length - next))
        }

        return (matches, isTruncated)
    }

    private func applySearchResults(
        _ matches: [ChapterSearchResult],
        isTruncated: Bool,
        selectFirst: Bool
    ) {
        searchResults = matches
        areSearchResultsTruncated = isTruncated
        if selectFirst || !matches.contains(where: { $0.id == selectedSearchResultID }) {
            selectedSearchResultID = matches.first?.id
        }
    }

    private func jumpToSearchResult(at index: Int) {
        guard index >= 0, index < searchResults.count else { return }
        let result = searchResults[index]
        selectedSearchResultID = result.id
        refreshSearchResults()
        config.onJumpToCharacterOffset?(result.range.location)
    }

    private func selectPreviousSearchResult() {
        guard !searchResults.isEmpty else { return }
        let cur = searchResults.firstIndex(where: { $0.id == selectedSearchResultID }) ?? 0
        jumpToSearchResult(at: (cur - 1 + searchResults.count) % searchResults.count)
    }

    private func selectNextSearchResult() {
        guard !searchResults.isEmpty else { return }
        let cur = searchResults.firstIndex(where: { $0.id == selectedSearchResultID }) ?? -1
        jumpToSearchResult(at: (cur + 1) % searchResults.count)
    }

    private func snippet(for range: NSRange, in text: String) -> String {
        let source = text as NSString
        let ctx = 26
        let start = max(0, range.location - ctx)
        let end = min(source.length, range.location + range.length + ctx)
        return source.substring(with: NSRange(location: start, length: end - start))
            .replacingOccurrences(of: "\n", with: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    // MARK: - Theme Circle

    private func themeCircle(type: ReadingThemeType, color: Color, borderColor: Color) -> some View {
        Button {
            preferences.themeType = type
        } label: {
            ZStack {
                Circle()
                    .fill(color)
                    .frame(width: 32, height: 32)
                    .overlay(Circle().stroke(borderColor, lineWidth: 1))
                if preferences.themeType == type {
                    Image(systemName: "checkmark")
                        .font(.system(size: 12, weight: .bold))
                        .foregroundStyle(type == .dark ? .white : .black)
                }
            }
        }
        .buttonStyle(.plain)
    }
}

// MARK: - View Extension

extension View {
    /// Applies the reader toolbar items into the system titlebar.
    func readerToolbar(config: ReaderToolbarConfig) -> some View {
        modifier(ReaderToolbarModifier(config: config))
    }
}

// MARK: - Bottom Overlay (page arrows + status bar)

struct ReaderBottomOverlay: View {
    let currentBookPage: Int
    let totalBookPages: Int
    let chapterRemainingPages: Int
    let readingProgress: Double

    var onPreviousPage: (() -> Void)?
    var onNextPage: (() -> Void)?

    @ObservedObject private var preferences = ReadingPreferences.shared
    @State private var isLeftArrowHovered = false
    @State private var isRightArrowHovered = false

    private var theme: ReadingTheme { preferences.currentTheme }

    private var clampedProgress: Double {
        min(max(readingProgress, 0), 1)
    }

    private var currentPageLabel: String {
        guard totalBookPages > 0 else { return "" }
        return "\(max(currentBookPage, 1)) / \(totalBookPages)" + L("reader.pageUnit")
    }

    var body: some View {
        ZStack {
            // Ghost page arrows
            HStack {
                pageArrowButton(direction: .previous)
                Spacer()
                pageArrowButton(direction: .next)
            }
            .padding(.horizontal, 12)

            // Bottom status
            VStack {
                Spacer()
                bottomStatusBar
            }
        }
    }

    // MARK: - Bottom Status Bar

    private var bottomStatusBar: some View {
        VStack(spacing: 6) {
            HStack {
                if chapterRemainingPages > 0 {
                    Text(L("reader.chapterRemaining", chapterRemainingPages))
                        .font(.system(size: 10))
                        .foregroundStyle(theme.textColor.opacity(0.36))
                }

                Spacer()

                if !currentPageLabel.isEmpty {
                    Text(currentPageLabel)
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(theme.textColor.opacity(0.4))
                }
            }
            .padding(.horizontal, 24)

            // Full-width thin progress bar
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Rectangle()
                        .fill(theme.textColor.opacity(0.06))
                    Rectangle()
                        .fill(theme.textColor.opacity(0.2))
                        .frame(width: geo.size.width * clampedProgress)
                }
            }
            .frame(height: 1.5)
        }
        .padding(.bottom, 8)
    }

    // MARK: - Page Arrows

    private enum PageArrowDirection { case previous, next }

    private func pageArrowButton(direction: PageArrowDirection) -> some View {
        let isHovered = direction == .previous ? isLeftArrowHovered : isRightArrowHovered
        let systemName = direction == .previous ? "chevron.left" : "chevron.right"

        return Button {
            if direction == .previous { onPreviousPage?() } else { onNextPage?() }
        } label: {
            Image(systemName: systemName)
                .font(.system(size: 16, weight: .medium))
                .foregroundStyle(theme.textColor.opacity(isHovered ? 0.5 : 0.15))
                .frame(width: 36, height: 72)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            withAnimation(.easeInOut(duration: 0.15)) {
                if direction == .previous {
                    isLeftArrowHovered = hovering
                } else {
                    isRightArrowHovered = hovering
                }
            }
        }
    }
}

// MARK: - Helpers

private extension Double {
    func rounded(toPlaces places: Int) -> Double {
        let divisor = pow(10.0, Double(places))
        return (self * divisor).rounded() / divisor
    }
}
