import XCTest
@testable import SubReader
@testable import ReaderBridge
@testable import ReaderModels

final class AppTests: XCTestCase {

    @MainActor
    func testChapterCacheSeparatesBooksWithSamePath() {
        let cache = ChapterCache()
        let firstKey = ChapterCacheKey(bookId: "book-a", chapterPath: "chapter-1.xhtml", themeHash: "theme")
        let secondKey = ChapterCacheKey(bookId: "book-b", chapterPath: "chapter-1.xhtml", themeHash: "theme")

        cache.set(key: firstKey, value: "first")
        cache.set(key: secondKey, value: "second")

        XCTAssertEqual(cache.get(key: firstKey) as? String, "first")
        XCTAssertEqual(cache.get(key: secondKey) as? String, "second")
    }

    @MainActor
    func testChapterCacheInvalidateRemovesOnlyMatchingBook() {
        let cache = ChapterCache()
        let firstKey = ChapterCacheKey(bookId: "book-a", chapterPath: "chapter-1.xhtml", themeHash: "theme")
        let secondKey = ChapterCacheKey(bookId: "book-b", chapterPath: "chapter-1.xhtml", themeHash: "theme")

        cache.set(key: firstKey, value: "first")
        cache.set(key: secondKey, value: "second")
        cache.invalidate(bookId: "book-a")

        XCTAssertNil(cache.get(key: firstKey))
        XCTAssertEqual(cache.get(key: secondKey) as? String, "second")
    }

    func testTxtContentStoreRemoveClearsSingleBook() {
        let store = TxtContentStore.shared
        store.clear()
        defer { store.clear() }

        store.store(bookId: "book-a", result: Self.sampleTxtParseResult(title: "A"))
        store.store(bookId: "book-b", result: Self.sampleTxtParseResult(title: "B"))
        store.remove(bookId: "book-a")

        XCTAssertNil(store.get(bookId: "book-a"))
        XCTAssertNotNil(store.get(bookId: "book-b"))
    }

    func testTxtContentStoreEvictsLeastRecentlyUsedBook() {
        let store = TxtContentStore.shared
        store.clear()
        defer { store.clear() }

        store.store(bookId: "book-1", result: Self.sampleTxtParseResult(title: "One"))
        store.store(bookId: "book-2", result: Self.sampleTxtParseResult(title: "Two"))
        store.store(bookId: "book-3", result: Self.sampleTxtParseResult(title: "Three"))
        store.store(bookId: "book-4", result: Self.sampleTxtParseResult(title: "Four"))

        _ = store.get(bookId: "book-1")

        store.store(bookId: "book-5", result: Self.sampleTxtParseResult(title: "Five"))

        XCTAssertNotNil(store.get(bookId: "book-1"))
        XCTAssertNil(store.get(bookId: "book-2"))
        XCTAssertNotNil(store.get(bookId: "book-5"))
    }

    @MainActor
    func testBookmarkManagerLoadsOnlyOnceUnlessForced() {
        let engine = CountingReaderEngine()
        let manager = BookmarkManager(engine: engine, bookId: "book-1")

        XCTAssertEqual(engine.listBookmarksCallCount, 1)

        manager.loadBookmarks()
        XCTAssertEqual(engine.listBookmarksCallCount, 1)

        manager.loadBookmarks(force: true)
        XCTAssertEqual(engine.listBookmarksCallCount, 2)
    }

    @MainActor
    func testAnnotationManagerLoadsOnlyOnceUnlessForced() {
        let engine = CountingReaderEngine()
        let manager = AnnotationManager(engine: engine, bookId: "book-1")

        XCTAssertEqual(engine.listAnnotationsCallCount, 1)

        manager.loadAnnotations()
        XCTAssertEqual(engine.listAnnotationsCallCount, 1)

        manager.loadAnnotations(force: true)
        XCTAssertEqual(engine.listAnnotationsCallCount, 2)
    }
}

private extension AppTests {
    static func sampleTxtParseResult(title: String) -> TxtParseResult {
        TxtParseResult(
            encoding: "utf-8",
            hadReplacements: false,
            chapters: [
                TxtChapter(
                    title: title,
                    nodes: [DomNode(nodeType: .text, text: title)]
                )
            ]
        )
    }
}

private final class CountingReaderEngine: @unchecked Sendable, ReaderEngineProtocol {
    var listBookmarksCallCount = 0
    var listAnnotationsCallCount = 0

    func initialize(dbPath: String, deviceId: String) -> Result<Void, ReaderError> { .success(()) }
    func destroy() -> Result<Void, ReaderError> { .success(()) }
    func openBook(data: Data) -> Result<Void, ReaderError> { .success(()) }
    func closeBook() -> Result<Void, ReaderError> { .success(()) }
    func getMetadata() -> Result<BookMetadata, ReaderError> { .failure(.notFound) }
    func getChapterContent(path: String) -> Result<[DomNode], ReaderError> { .failure(.notFound) }
    func getToc() -> Result<[TocEntry], ReaderError> { .success([]) }
    func getSpine() -> Result<[String], ReaderError> { .success([]) }
    func getCoverImage(coverId: String) -> Result<Data, ReaderError> { .failure(.notFound) }
    func resolveTocHref(href: String) -> Result<Int, ReaderError> { .failure(.notFound) }
    func parseTxt(data: Data) -> Result<TxtParseResult, ReaderError> { .failure(.parseFailed) }
    func getProgress(bookId: String) -> Result<ReadingProgress, ReaderError> { .failure(.notFound) }
    func updateProgress(bookId: String, cfi: String, percentage: Double, hlcTs: UInt64) -> Result<Void, ReaderError> { .success(()) }
    func addBookmark(_ bookmark: Bookmark) -> Result<Void, ReaderError> { .success(()) }
    func deleteBookmark(id: String, hlcTs: UInt64) -> Result<Void, ReaderError> { .success(()) }
    func listBookmarks(bookId: String) -> Result<[Bookmark], ReaderError> {
        listBookmarksCallCount += 1
        return .success([])
    }
    func addAnnotation(_ annotation: Annotation) -> Result<Void, ReaderError> { .success(()) }
    func deleteAnnotation(id: String, hlcTs: UInt64) -> Result<Void, ReaderError> { .success(()) }
    func listAnnotations(bookId: String) -> Result<[Annotation], ReaderError> {
        listAnnotationsCallCount += 1
        return .success([])
    }
}