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

    /// Initialize the engine with a database path, device identifier, and optional backend URL.
    func initialize(dbPath: String, deviceId: String, baseURL: String?) -> Result<Void, ReaderError>

    /// Destroy the engine and release all resources.
    func destroy() -> Result<Void, ReaderError>

    // MARK: - Authentication

    /// Register a new user account. Returns the user ID or backend payload.
    func authRegister(username: String, email: String, password: String) -> Result<String, ReaderError>

    /// Login with credentials and optional device metadata. Returns token JSON.
    func authLoginWithMetadata(
        credential: String,
        password: String,
        deviceName: String?,
        platform: String?
    ) -> Result<String, ReaderError>

    /// Logout the current session.
    func authLogout() -> Result<Void, ReaderError>

    /// Get current auth state code.
    func authGetState() -> Int32

    /// Change the current user's password.
    func authChangePassword(oldPassword: String, newPassword: String) -> Result<Void, ReaderError>

    /// List devices for the current user.
    func authListDevices() -> Result<String, ReaderError>

    /// Remove a device from the current user's device list.
    func authRemoveDevice(deviceId: String) -> Result<Void, ReaderError>

    /// Set the auth state change callback.
    func setAuthCallback(_ callback: (@convention(c) (Int32) -> Void)?) -> Result<Void, ReaderError>

    // MARK: - Sync

    /// Perform a full sync (push + pull).
    func syncFull() -> Result<Void, ReaderError>

    /// Start the background sync scheduler.
    func syncStartScheduler() -> Result<Void, ReaderError>

    /// Stop the background sync scheduler.
    func syncStopScheduler() -> Result<Void, ReaderError>

    /// Set the sync state change callback.
    func setSyncCallback(_ callback: (@convention(c) (Int32) -> Void)?) -> Result<Void, ReaderError>

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

    /// Resolve a TOC entry href to a spine index.
    /// Returns the 0-based spine index, or .failure(.notFound) if no match.
    func resolveTocHref(href: String) -> Result<Int, ReaderError>

    // MARK: - TXT Operations

    /// Parse a TXT file from raw bytes with chapter splitting.
    /// This is a stateless operation — no need to call openBook first.
    func parseTxt(data: Data) -> Result<TxtParseResult, ReaderError>

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
