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

    /// Whether this book is a plain-text file handled natively (no Rust engine).
    var isPlainText: Bool { metadata.format == .txt }
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

    // MARK: - TXT Import (native Swift, no Rust engine)

    /// Import a plain-text file entirely in Swift.
    private func importTxtBook(data: Data, fileURL: URL?) {
        isLoading = true
        currentError = nil

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }

            // Detect encoding: try UTF-8 first, then fallback to common CJK encodings
            let content: String
            if let utf8 = String(data: data, encoding: .utf8) {
                content = utf8
            } else if let gb = String(data: data, encoding: .init(rawValue: CFStringConvertEncodingToNSStringEncoding(CFStringEncoding(CFStringEncodings.GB_18030_2000.rawValue)))) {
                content = gb
            } else if let latin = String(data: data, encoding: .isoLatin1) {
                content = latin
            } else {
                DispatchQueue.main.async {
                    self.isLoading = false
                    self.currentError = .parseFailed
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
                fileURL: fileURL
            )

            // Store parsed content for later reading
            TxtContentStore.shared.store(bookId: bookId, content: content)

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
            // Close the book after extracting metadata (we'll reopen when reading)
            let _ = self.engine.closeBook()

            switch metaResult {
            case .success(let metadata):
                let book = LibraryBook(
                    id: metadata.id,
                    metadata: metadata,
                    progress: 0.0,
                    coverData: nil,
                    fileURL: fileURL
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
}
