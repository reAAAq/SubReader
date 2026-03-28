// PaginatedReaderView — Paginated reading container supporting single and dual page modes.
//
// Shared by both EPUB and TXT readers. Handles pagination, page navigation,
// dual/single page layout switching, and page number display.

import SwiftUI
import AppKit

/// Paginated reader container that displays content in fixed pages (not scrolling).
struct PaginatedReaderView: View {
    /// The full attributed string for the current chapter.
    let attributedString: NSAttributedString
    /// Total number of pages (set after pagination).
    @Binding var totalPages: Int
    /// Current page index (zero-based).
    @Binding var currentPageIndex: Int

    /// Character offset to restore when content changes.
    var restoreCharacterOffset: Int? = nil
    /// Callback when user requests next chapter.
    var onNextChapter: (() -> Void)?
    /// Callback when user requests previous chapter.
    var onPreviousChapter: (() -> Void)?
    /// Callback when user taps the center area (to show toolbar).
    var onCenterTap: (() -> Void)?
    /// Callback when page changes (reports character offset for progress saving).
    var onPageChange: ((Int) -> Void)?
    /// Callback when pagination is recalculated.
    var onPagesUpdated: (([PageSlice], CGSize) -> Void)?

    @ObservedObject private var layoutManager: PageLayoutManager
    @ObservedObject private var preferences: ReadingPreferences

    /// Paginated page slices.
    @State private var pages: [PageSlice] = []
    /// Available page size from GeometryReader.
    @State private var availableSize: CGSize = .zero
    /// Page transition animation ID.
    @State private var pageTransitionID = UUID()

    init(
        attributedString: NSAttributedString,
        totalPages: Binding<Int>,
        currentPageIndex: Binding<Int>,
        layoutManager: PageLayoutManager,
        preferences: ReadingPreferences,
        restoreCharacterOffset: Int? = nil,
        onNextChapter: (() -> Void)? = nil,
        onPreviousChapter: (() -> Void)? = nil,
        onCenterTap: (() -> Void)? = nil,
        onPageChange: ((Int) -> Void)? = nil,
        onPagesUpdated: (([PageSlice], CGSize) -> Void)? = nil
    ) {
        self.attributedString = attributedString
        self._totalPages = totalPages
        self._currentPageIndex = currentPageIndex
        self.layoutManager = layoutManager
        self.preferences = preferences
        self.restoreCharacterOffset = restoreCharacterOffset
        self.onNextChapter = onNextChapter
        self.onPreviousChapter = onPreviousChapter
        self.onCenterTap = onCenterTap
        self.onPageChange = onPageChange
        self.onPagesUpdated = onPagesUpdated
    }

    private var theme: ReadingTheme {
        preferences.currentTheme
    }

    private var bgNSColor: NSColor {
        NSColor(theme.backgroundColor)
    }

    private var textNSColor: NSColor {
        NSColor(theme.textColor)
    }

    private var pageNumberColor: NSColor {
        NSColor(theme.textColor).withAlphaComponent(0.5)
    }

    /// The page size for a single page in the current layout mode.
    private var singlePageSize: CGSize {
        if layoutManager.isDualPage {
            // Each page gets half the width minus the center gap
            let pageWidth = (availableSize.width - 1) / 2.0
            return CGSize(width: pageWidth, height: availableSize.height)
        } else {
            // Single page mode: constrain width to 680px max, centered
            let pageWidth = min(availableSize.width, 680)
            return CGSize(width: pageWidth, height: availableSize.height)
        }
    }

    var body: some View {
        GeometryReader { geo in
            ZStack {
                // Background
                Color(nsColor: bgNSColor)

                // Page content with transition animation
                if pages.isEmpty {
                    // Empty or loading state
                    Color.clear
                } else if layoutManager.isDualPage {
                    dualPageLayout
                        .id(pageTransitionID)
                        .transition(.opacity)
                } else {
                    singlePageLayout
                        .id(pageTransitionID)
                        .transition(.opacity)
                }

                // Tap zones for navigation
                tapZones
            }
            .onChange(of: geo.size) { _, newSize in
                if newSize != availableSize {
                    availableSize = newSize
                    repaginate(preservePosition: true)
                }
            }
            .onAppear {
                availableSize = geo.size
                repaginate(preservePosition: false)
            }
        }
        .onChange(of: attributedString) { _, _ in
            repaginate(preservePosition: false)
        }
        .onChange(of: layoutManager.isDualPage) { _, _ in
            repaginate(preservePosition: true)
        }
        .onKeyPress(.leftArrow) {
            goToPreviousPage()
            return .handled
        }
        .onKeyPress(.rightArrow) {
            goToNextPage()
            return .handled
        }
    }

    // MARK: - Single Page Layout

    private var singlePageLayout: some View {
        HStack {
            Spacer(minLength: 0)
            if currentPageIndex < pages.count {
                PageRenderView(
                    attributedString: pages[currentPageIndex].attributedSubstring,
                    backgroundColor: bgNSColor,
                    pageNumber: nil,
                    pageNumberColor: pageNumberColor
                )
                .frame(width: singlePageSize.width, height: singlePageSize.height)
            }
            Spacer(minLength: 0)
        }
    }

    // MARK: - Dual Page Layout

    private var dualPageLayout: some View {
        HStack(spacing: 0) {
            // Left page (even index)
            let leftIndex = dualLeftPageIndex
            if leftIndex < pages.count {
                PageRenderView(
                    attributedString: pages[leftIndex].attributedSubstring,
                    backgroundColor: bgNSColor,
                    pageNumber: nil,
                    pageNumberColor: pageNumberColor
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                // Empty left page
                Color(nsColor: bgNSColor)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }

            // Center divider
            Rectangle()
                .fill(Color(nsColor: bgNSColor).opacity(0.3))
                .frame(width: 1)

            // Right page (odd index)
            let rightIndex = leftIndex + 1
            if rightIndex < pages.count {
                PageRenderView(
                    attributedString: pages[rightIndex].attributedSubstring,
                    backgroundColor: bgNSColor,
                    pageNumber: nil,
                    pageNumberColor: pageNumberColor
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                // Empty right page (chapter boundary)
                Color(nsColor: bgNSColor)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }

    /// In dual page mode, the left page always shows an even-indexed page.
    private var dualLeftPageIndex: Int {
        // Ensure left page is always even
        return (currentPageIndex / 2) * 2
    }

    // MARK: - Tap Zones

    private var tapZones: some View {
        HStack(spacing: 0) {
            // Left 1/3: previous page
            Color.clear
                .contentShape(Rectangle())
                .onTapGesture {
                    goToPreviousPage()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)

            // Center 1/3: show toolbar
            Color.clear
                .contentShape(Rectangle())
                .onTapGesture {
                    onCenterTap?()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)

            // Right 1/3: next page
            Color.clear
                .contentShape(Rectangle())
                .onTapGesture {
                    goToNextPage()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    // MARK: - Navigation

    private func goToNextPage() {
        let step = layoutManager.isDualPage ? 2 : 1
        let nextIndex = currentPageIndex + step

        if nextIndex < pages.count {
            withAnimation(.easeInOut(duration: 0.18)) {
                pageTransitionID = UUID()
                currentPageIndex = nextIndex
            }
            onPageChange?(PaginationEngine.characterOffset(forPageIndex: currentPageIndex, in: pages))
        } else {
            // End of chapter — request next chapter
            onNextChapter?()
        }
    }

    private func goToPreviousPage() {
        let step = layoutManager.isDualPage ? 2 : 1
        let prevIndex = currentPageIndex - step

        if prevIndex >= 0 {
            withAnimation(.easeInOut(duration: 0.18)) {
                pageTransitionID = UUID()
                currentPageIndex = prevIndex
            }
            onPageChange?(PaginationEngine.characterOffset(forPageIndex: currentPageIndex, in: pages))
        } else if currentPageIndex > 0 {
            withAnimation(.easeInOut(duration: 0.18)) {
                pageTransitionID = UUID()
                currentPageIndex = 0
            }
            onPageChange?(0)
        } else {
            // Beginning of chapter — request previous chapter
            onPreviousChapter?()
        }
    }

    // MARK: - Pagination

    private func repaginate(preservePosition: Bool) {
        guard availableSize.width > 0, availableSize.height > 0 else { return }

        // Save current character offset before repagination.
        let targetOffset: Int?
        if preservePosition, currentPageIndex < pages.count {
            targetOffset = pages[currentPageIndex].textRange.location
        } else {
            targetOffset = restoreCharacterOffset
        }

        let pageSize = singlePageSize
        let newPages = PaginationEngine.paginate(
            attributedString: attributedString,
            pageSize: pageSize
        )

        pages = newPages
        totalPages = newPages.count

        // Restore position.
        if let offset = targetOffset, !newPages.isEmpty {
            currentPageIndex = PaginationEngine.pageIndex(forCharacterOffset: offset, in: newPages)
        } else if currentPageIndex >= newPages.count {
            currentPageIndex = max(0, newPages.count - 1)
        }

        onPagesUpdated?(newPages, pageSize)
        if !newPages.isEmpty {
            onPageChange?(PaginationEngine.characterOffset(forPageIndex: currentPageIndex, in: newPages))
        }
    }
}