// BridgeTests — Unit tests for the ReaderBridge module.

import XCTest
@testable import ReaderBridge
@testable import ReaderModels

final class ReaderErrorTests: XCTestCase {

    func testErrorCodeMapping() {
        XCTAssertNil(ReaderError.from(code: 0))
        XCTAssertEqual(ReaderError.from(code: -1), .nullPointer)
        XCTAssertEqual(ReaderError.from(code: -2), .invalidUtf8)
        XCTAssertEqual(ReaderError.from(code: -3), .parseFailed)
        XCTAssertEqual(ReaderError.from(code: -4), .storage)
        XCTAssertEqual(ReaderError.from(code: -5), .notFound)
        XCTAssertEqual(ReaderError.from(code: -6), .alreadyInit)
        XCTAssertEqual(ReaderError.from(code: -7), .notInit)
        XCTAssertEqual(ReaderError.from(code: -98), .panic)
        XCTAssertEqual(ReaderError.from(code: -99), .unknown)
    }

    func testUnknownErrorCode() {
        XCTAssertEqual(ReaderError.from(code: -42), .unknown)
    }

    func testErrorDescriptions() {
        for error in [ReaderError.nullPointer, .invalidUtf8, .parseFailed, .storage,
                      .notFound, .alreadyInit, .notInit, .panic, .unknown] {
            XCTAssertNotNil(error.errorDescription)
            XCTAssertFalse(error.errorDescription!.isEmpty)
        }
    }
}

final class MockEngineTests: XCTestCase {

    func testMockEngineDefaultData() {
        let mock = MockReaderEngine()
        let metaResult = mock.getMetadata()
        XCTAssertNotNil(try? metaResult.get())
    }

    func testMockEngineFailMode() {
        let mock = MockReaderEngine()
        mock.shouldFail = true
        let result = mock.initialize(dbPath: "/tmp/test.db", deviceId: "test")
        XCTAssertThrowsError(try result.get())
    }

    func testMockBookmarkCRUD() {
        let mock = MockReaderEngine()
        let bookmark = Bookmark(id: "bm-1", bookId: "book-1", cfiPosition: "/6/4", title: "Test", createdAt: 1000)

        _ = mock.addBookmark(bookmark)
        let list = try! mock.listBookmarks(bookId: "book-1").get()
        XCTAssertEqual(list.count, 1)

        _ = mock.deleteBookmark(id: "bm-1", hlcTs: 2000)
        let list2 = try! mock.listBookmarks(bookId: "book-1").get()
        XCTAssertEqual(list2.count, 0)
    }
}
