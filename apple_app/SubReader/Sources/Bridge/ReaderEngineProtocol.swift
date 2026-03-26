// ReaderEngineProtocol — Abstract interface for the reader engine.
//
// Business logic depends ONLY on this protocol, never on RustCore directly.
// This enables dependency injection and mock testing.

import Foundation
import ReaderModels

/// Protocol defining all reader engine operations.
/// All methods are async and return Result types for explicit error handling.
public protocol ReaderEngineProtocol: Sendable {

    // MARK: - Engine Lifecycle

    /// Initialize the engine with a database path and device identifier.
    func initialize(dbPath: String, deviceId: String) -> Result<Void, ReaderError>

    /// Destroy the engine and release all resources.
    func destroy() -> Result<Void, ReaderError>

    // MARK: - Book Operations

    /// Open an EPUB book from raw file data.
    func openBook(data: Data) -> Result<Void, ReaderError>

    /// Close the currently opened book.
    func closeBook() -> Result<Void, ReaderError>

    /// Get metadata for the currently opened book.
    func getMetadata() -> Result<BookMetadata, ReaderError>

    /// Get chapter content as a DOM tree.
    func getChapterContent(path: String) -> Result<[DomNode], ReaderError>

    /// Get the table of contents.
    func getToc() -> Result<[TocEntry], ReaderError>

    /// Get the spine (ordered list of content document paths).
    func getSpine() -> Result<[String], ReaderError>

    /// Get the cover image data by manifest item id.
    func getCoverImage(coverId: String) -> Result<Data, ReaderError>

    // MARK: - Progress

    /// Get reading progress for a book.
    func getProgress(bookId: String) -> Result<ReadingProgress, ReaderError>

    /// Update reading progress.
    func updateProgress(bookId: String, cfi: String, percentage: Double, hlcTs: UInt64) -> Result<Void, ReaderError>

    // MARK: - Bookmarks

    /// Add a bookmark.
    func addBookmark(_ bookmark: Bookmark) -> Result<Void, ReaderError>

    /// Delete a bookmark by ID.
    func deleteBookmark(id: String, hlcTs: UInt64) -> Result<Void, ReaderError>

    /// List all bookmarks for a book.
    func listBookmarks(bookId: String) -> Result<[Bookmark], ReaderError>

    // MARK: - Annotations

    /// Add an annotation.
    func addAnnotation(_ annotation: Annotation) -> Result<Void, ReaderError>

    /// Delete an annotation by ID.
    func deleteAnnotation(id: String, hlcTs: UInt64) -> Result<Void, ReaderError>

    /// List all annotations for a book.
    func listAnnotations(bookId: String) -> Result<[Annotation], ReaderError>
}
