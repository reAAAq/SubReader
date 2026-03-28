// BookmarkManager — Bookmark management with O(1) lookup.

import Foundation
import ReaderModels
import ReaderBridge

/// Manages bookmarks for the current book with in-memory cache for fast lookup.
@MainActor
final class BookmarkManager: ObservableObject {

    @Published var bookmarks: [Bookmark] = []

    /// Set of CFI positions for O(1) "is bookmarked?" checks.
    private var bookmarkedPositions: Set<String> = []

    private let engine: any ReaderEngineProtocol
    private let bookId: String
    private var hasLoadedBookmarks = false
    private var isLoadingBookmarks = false

    init(engine: any ReaderEngineProtocol, bookId: String) {
        self.engine = engine
        self.bookId = bookId
        loadBookmarks()
    }

    /// Check if a position is bookmarked (O(1)).
    func isBookmarked(cfi: String) -> Bool {
        bookmarkedPositions.contains(cfi)
    }

    /// Add a bookmark at the given position.
    func addBookmark(cfi: String, title: String?) {
        let bookmark = Bookmark(
            id: UUID().uuidString,
            bookId: bookId,
            cfiPosition: cfi,
            title: title,
            createdAt: UInt64(Date().timeIntervalSince1970)
        )

        let result = engine.addBookmark(bookmark)
        if case .success = result {
            bookmarks.append(bookmark)
            bookmarkedPositions.insert(cfi)
        }
    }

    /// Delete a bookmark by ID.
    func deleteBookmark(id: String) {
        let hlcTs = UInt64(Date().timeIntervalSince1970)
        let result = engine.deleteBookmark(id: id, hlcTs: hlcTs)
        if case .success = result {
            if let removed = bookmarks.first(where: { $0.id == id }) {
                bookmarkedPositions.remove(removed.cfiPosition)
            }
            bookmarks.removeAll { $0.id == id }
        }
    }

    /// Toggle bookmark at position.
    func toggleBookmark(cfi: String, title: String?) {
        if let existing = bookmarks.first(where: { $0.cfiPosition == cfi }) {
            deleteBookmark(id: existing.id)
        } else {
            addBookmark(cfi: cfi, title: title)
        }
    }

    /// Reload bookmarks from engine.
    func loadBookmarks(force: Bool = false) {
        guard force || (!hasLoadedBookmarks && !isLoadingBookmarks) else { return }
        isLoadingBookmarks = true
        defer { isLoadingBookmarks = false }

        let result = engine.listBookmarks(bookId: bookId)
        if case .success(let list) = result {
            bookmarks = list
            bookmarkedPositions = Set(list.map(\.cfiPosition))
            hasLoadedBookmarks = true
        }
    }
}
