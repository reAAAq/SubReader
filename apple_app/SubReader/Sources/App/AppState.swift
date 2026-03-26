// AppState — Global observable state for the SubReader app.
//
// Holds the current navigation state, open books, and engine reference.

import SwiftUI
import UniformTypeIdentifiers
import ReaderModels
import ReaderBridge

/// Represents the current navigation destination.
enum NavigationDestination: Hashable {
    case library
    case reader(bookId: String)
}

/// Library item representing an imported book.
struct LibraryBook: Identifiable, Hashable {
    let id: String
    let metadata: BookMetadata
    var progress: Double
    var coverData: Data?
    var fileURL: URL?
    /// Raw EPUB data kept in memory so we can reopen the book for reading.
    var bookData: Data?

    /// Whether this book is a plain-text file handled natively (no Rust engine).
    var isPlainText: Bool { metadata.format == .txt }

    static func == (lhs: LibraryBook, rhs: LibraryBook) -> Bool {
        lhs.id == rhs.id
    }

    func hash(into hasher: inout Hasher) {
        hasher.combine(id)
    }
}

/// Global application state.
@MainActor
final class AppState: ObservableObject {

    // MARK: - Published State

    /// Current navigation destination.
    @Published var currentDestination: NavigationDestination = .library

    /// All imported books in the library.
    @Published var libraryBooks: [LibraryBook] = []

    /// Currently reading book ID (if any).
    @Published var currentBookId: String?

    /// Whether the sidebar (TOC) is visible.
    @Published var isSidebarVisible = true

    /// Loading state for async operations.
    @Published var isLoading = false

    /// Current error to display (if any).
    @Published var currentError: ReaderError?

    // MARK: - Engine Reference

    let engine: any ReaderEngineProtocol

    // MARK: - Init

    init(engine: any ReaderEngineProtocol) {
        self.engine = engine
    }

    // MARK: - Book Operations

    /// Supported file extensions for import.
    static let supportedExtensions: Set<String> = ["epub", "txt"]

    /// Check whether a URL has a supported file extension.
    static func isSupportedFile(_ url: URL) -> Bool {
        supportedExtensions.contains(url.pathExtension.lowercased())
    }

    /// Import a book from file data.
    func importBook(data: Data, fileURL: URL? = nil) {
        // Detect format from file extension
        let ext = fileURL?.pathExtension.lowercased() ?? ""
        if ext == "txt" {
            importTxtBook(data: data, fileURL: fileURL)
        } else {
            importEpubBook(data: data, fileURL: fileURL)
        }
    }

    // MARK: - TXT Import (via Rust engine)

    /// Import a plain-text file using the Rust engine for parsing.
    private func importTxtBook(data: Data, fileURL: URL?) {
        isLoading = true
        currentError = nil

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }

            // Parse via Rust engine for high-performance encoding detection
            let parseResult: TxtParseResult
            switch self.engine.parseTxt(data: data) {
            case .success(let result):
                parseResult = result
            case .failure(let error):
                DispatchQueue.main.async {
                    self.isLoading = false
                    self.currentError = error
                }
                return
            }

            // Build metadata from filename
            let fileName = fileURL?.deletingPathExtension().lastPathComponent ?? "Untitled"
            let bookId = "txt-" + (fileURL?.absoluteString.data(using: .utf8)?.base64EncodedString().prefix(32).description ?? UUID().uuidString)

            let metadata = BookMetadata(
                id: bookId,
                title: fileName,
                authors: [],
                format: .txt,
                fileSize: UInt64(data.count)
            )

            let book = LibraryBook(
                id: bookId,
                metadata: metadata,
                progress: 0.0,
                coverData: nil,
                fileURL: fileURL,
                bookData: nil
            )

            // Cache parsed result for later reading
            TxtContentStore.shared.store(bookId: bookId, result: parseResult)

            DispatchQueue.main.async {
                if !self.libraryBooks.contains(where: { $0.id == book.id }) {
                    self.libraryBooks.append(book)
                }
                self.isLoading = false
            }
        }
    }

    // MARK: - EPUB Import (via Rust engine)

    private func importEpubBook(data: Data, fileURL: URL?) {
        isLoading = true
        currentError = nil

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }

            let openResult = self.engine.openBook(data: data)
            switch openResult {
            case .failure(let error):
                DispatchQueue.main.async {
                    self.isLoading = false
                    self.currentError = error
                }
                return
            case .success:
                break
            }

            let metaResult = self.engine.getMetadata()

            // Try to extract cover image while the book is still open
            var coverData: Data? = nil
            if case .success(let meta) = metaResult, let coverId = meta.coverImageRef {
                if case .success(let imgData) = self.engine.getCoverImage(coverId: coverId) {
                    coverData = imgData
                }
            }

            // Close the book after extracting metadata and cover
            let _ = self.engine.closeBook()

            switch metaResult {
            case .success(let metadata):
                let book = LibraryBook(
                    id: metadata.id,
                    metadata: metadata,
                    progress: 0.0,
                    coverData: coverData,
                    fileURL: fileURL,
                    bookData: data
                )
                DispatchQueue.main.async {
                    // Avoid duplicates
                    if !self.libraryBooks.contains(where: { $0.id == book.id }) {
                        self.libraryBooks.append(book)
                    }
                    self.isLoading = false
                }
            case .failure(let error):
                DispatchQueue.main.async {
                    self.isLoading = false
                    self.currentError = error
                }
            }
        }
    }

    /// Remove a book from the library.
    func removeBook(id: String) {
        libraryBooks.removeAll { $0.id == id }
        if currentBookId == id {
            currentBookId = nil
            currentDestination = .library
        }
    }

    /// Open a book for reading.
    func openBook(_ book: LibraryBook) {
        currentBookId = book.id
        currentDestination = .reader(bookId: book.id)
    }

    /// Reopen the EPUB file in the Rust engine for reading.
    /// Must be called before accessing chapter content, TOC, etc.
    func reopenEpubForReading(_ book: LibraryBook) -> Bool {
        // Try bookData first, then fall back to fileURL
        let data: Data?
        if let bd = book.bookData {
            data = bd
        } else if let url = book.fileURL {
            data = try? Data(contentsOf: url)
        } else {
            data = nil
        }

        guard let epubData = data else { return false }

        if case .success = engine.openBook(data: epubData) {
            return true
        }
        return false
    }
}
