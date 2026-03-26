// MockReaderEngine — Mock implementation for SwiftUI Previews and unit tests.
//
// Returns predefined data without any FFI calls.

import Foundation
import ReaderModels

/// Mock engine for testing and SwiftUI previews.
public final class MockReaderEngine: ReaderEngineProtocol, @unchecked Sendable {

    public var shouldFail = false
    public var mockMetadata: BookMetadata?
    public var mockChapterContent: [DomNode] = []
    public var mockProgress: ReadingProgress?
    public var mockBookmarks: [Bookmark] = []
    public var mockAnnotations: [Annotation] = []

    public init() {
        // Set up default mock data
        mockMetadata = BookMetadata(
            id: "mock-book-1",
            title: "Mock Book Title",
            authors: ["Mock Author"],
            language: "en",
            format: .epub
        )

        mockChapterContent = [
            DomNode(
                nodeType: .heading(level: 1),
                cfiAnchor: "/6/4!/4/2",
                children: [
                    DomNode(nodeType: .text, text: "Chapter 1: Introduction")
                ]
            ),
            DomNode(
                nodeType: .paragraph,
                cfiAnchor: "/6/4!/4/4",
                children: [
                    DomNode(nodeType: .text, text: "This is a sample paragraph for preview purposes.")
                ]
            )
        ]

        mockProgress = ReadingProgress(
            bookId: "mock-book-1",
            cfiPosition: "/6/4!/4/2:0",
            percentage: 25.0,
            hlcTimestamp: UInt64(Date().timeIntervalSince1970)
        )
    }

    private func failOrSucceed<T>(_ value: T) -> Result<T, ReaderError> {
        shouldFail ? .failure(.unknown) : .success(value)
    }

    // MARK: - ReaderEngineProtocol

    public func initialize(dbPath: String, deviceId: String) -> Result<Void, ReaderError> {
        failOrSucceed(())
    }

    public func destroy() -> Result<Void, ReaderError> {
        failOrSucceed(())
    }

    public func openBook(data: Data) -> Result<Void, ReaderError> {
        failOrSucceed(())
    }

    public func closeBook() -> Result<Void, ReaderError> {
        failOrSucceed(())
    }

    public func getMetadata() -> Result<BookMetadata, ReaderError> {
        guard let metadata = mockMetadata else { return .failure(.notFound) }
        return failOrSucceed(metadata)
    }

    public func getChapterContent(path: String) -> Result<[DomNode], ReaderError> {
        failOrSucceed(mockChapterContent)
    }

    public func getProgress(bookId: String) -> Result<ReadingProgress, ReaderError> {
        guard let progress = mockProgress else { return .failure(.notFound) }
        return failOrSucceed(progress)
    }

    public func updateProgress(bookId: String, cfi: String, percentage: Double, hlcTs: UInt64) -> Result<Void, ReaderError> {
        if !shouldFail {
            mockProgress = ReadingProgress(bookId: bookId, cfiPosition: cfi, percentage: percentage, hlcTimestamp: hlcTs)
        }
        return failOrSucceed(())
    }

    public func addBookmark(_ bookmark: Bookmark) -> Result<Void, ReaderError> {
        if !shouldFail { mockBookmarks.append(bookmark) }
        return failOrSucceed(())
    }

    public func deleteBookmark(id: String, hlcTs: UInt64) -> Result<Void, ReaderError> {
        if !shouldFail { mockBookmarks.removeAll { $0.id == id } }
        return failOrSucceed(())
    }

    public func listBookmarks(bookId: String) -> Result<[Bookmark], ReaderError> {
        failOrSucceed(mockBookmarks.filter { $0.bookId == bookId })
    }

    public func addAnnotation(_ annotation: Annotation) -> Result<Void, ReaderError> {
        if !shouldFail { mockAnnotations.append(annotation) }
        return failOrSucceed(())
    }

    public func deleteAnnotation(id: String, hlcTs: UInt64) -> Result<Void, ReaderError> {
        if !shouldFail { mockAnnotations.removeAll { $0.id == id } }
        return failOrSucceed(())
    }

    public func listAnnotations(bookId: String) -> Result<[Annotation], ReaderError> {
        failOrSucceed(mockAnnotations.filter { $0.bookId == bookId })
    }
}
