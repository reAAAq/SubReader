// AppState — Global observable state for the SubReader app.
//
// Holds the current navigation state, open books, and engine reference.

import SwiftUI
import UniformTypeIdentifiers
import ReaderModels
import ReaderBridge

/// User-facing error payload for app-level alerts.
struct AppDisplayError: LocalizedError {
    let message: String

    init(message: String) {
        self.message = message
    }

    init(_ error: any Error) {
        if let localizedError = error as? LocalizedError,
           let description = localizedError.errorDescription,
           !description.isEmpty {
            self.message = description
        } else {
            self.message = error.localizedDescription
        }
    }

    static func fileReadFailed(url: URL, underlyingError: any Error) -> AppDisplayError {
        let baseMessage: String
        if let localizedError = underlyingError as? LocalizedError,
           let description = localizedError.errorDescription,
           !description.isEmpty {
            baseMessage = description
        } else {
            baseMessage = underlyingError.localizedDescription
        }
        return AppDisplayError(message: "Failed to read \(url.lastPathComponent): \(baseMessage)")
    }

    var errorDescription: String? {
        message
    }
}

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
    /// Optional fallback raw book data retained only when no durable file URL is available.
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

    /// Currently reading book ID derived from navigation state.
    var currentBookId: String? {
        guard case .reader(let bookId) = currentDestination else { return nil }
        return bookId
    }

    /// Whether the app is currently presenting a reader.
    var isReaderActive: Bool {
        currentBookId != nil
    }

    /// Whether the sidebar (TOC) is visible.
    @Published var isSidebarVisible = true

    /// Loading state for async operations.
    @Published var isLoading = false

    /// Current error to display (if any).
    @Published var currentError: AppDisplayError?

    /// Search text for filtering library books.
    @Published var searchText: String = ""

    /// Filtered books based on search text.
    var filteredBooks: [LibraryBook] {
        guard !searchText.isEmpty else { return libraryBooks }
        let query = searchText.lowercased()
        return libraryBooks.filter { book in
            book.metadata.title.lowercased().contains(query) ||
            book.metadata.authors.joined(separator: " ").lowercased().contains(query)
        }
    }

    // MARK: - Engine Reference

    let engine: any ReaderEngineProtocol
    private var activeImportOperations = 0
    private let chapterCache: ChapterCache
    private let coverCache: CoverImageCache

    // MARK: - Init

    init(
        engine: any ReaderEngineProtocol,
        chapterCache: ChapterCache,
        coverCache: CoverImageCache
    ) {
        self.engine = engine
        self.chapterCache = chapterCache
        self.coverCache = coverCache
    }

    // MARK: - Book Operations

    /// Supported file extensions for import.
    static let supportedExtensions: Set<String> = ["epub", "txt"]

    /// Check whether a URL has a supported file extension.
    nonisolated static func isSupportedFile(_ url: URL) -> Bool {
        supportedExtensions.contains(url.pathExtension.lowercased())
    }

    /// Import a book from file data.
    func importBook(data: Data, fileURL: URL? = nil) {
        beginImportOperation(resetError: true)
        importBook(data: data, fileURL: fileURL, usesExistingLoadingState: true)
    }

    /// Import a book from file URL without blocking the main thread.
    func importBook(from fileURL: URL) {
        guard Self.isSupportedFile(fileURL) else { return }

        beginImportOperation(resetError: true)

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }

            let readResult = Self.readBookData(from: fileURL)
            DispatchQueue.main.async {
                switch readResult {
                case .success(let data):
                    self.importBook(data: data, fileURL: fileURL, usesExistingLoadingState: true)
                case .failure(let error):
                    self.currentError = error
                    self.endImportOperation()
                }
            }
        }
    }

    private func importBook(data: Data, fileURL: URL?, usesExistingLoadingState: Bool) {
        if !usesExistingLoadingState {
            beginImportOperation(resetError: true)
        }

        let ext = fileURL?.pathExtension.lowercased() ?? ""
        if ext == "txt" {
            importTxtBook(data: data, fileURL: fileURL)
        } else {
            importEpubBook(data: data, fileURL: fileURL)
        }
    }

    private func beginImportOperation(resetError: Bool) {
        if resetError {
            currentError = nil
        }
        activeImportOperations += 1
        isLoading = activeImportOperations > 0
    }

    private func endImportOperation() {
        activeImportOperations = max(0, activeImportOperations - 1)
        isLoading = activeImportOperations > 0
    }

    private nonisolated static func readBookData(from fileURL: URL) -> Result<Data, AppDisplayError> {
        let didStartAccessing = fileURL.startAccessingSecurityScopedResource()
        defer {
            if didStartAccessing {
                fileURL.stopAccessingSecurityScopedResource()
            }
        }

        do {
            let data = try Data(contentsOf: fileURL, options: [.mappedIfSafe])
            return .success(data)
        } catch {
            return .failure(.fileReadFailed(url: fileURL, underlyingError: error))
        }
    }

    private nonisolated static func importedBooksDirectory() -> URL {
        let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let directory = appSupport.appendingPathComponent("SubReader/imported-books", isDirectory: true)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }

    private nonisolated static func sanitizedStorageName(for identifier: String) -> String {
        let sanitized = identifier.unicodeScalars.map { scalar -> Character in
            let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-_"))
            return allowed.contains(scalar) ? Character(scalar) : "_"
        }
        let name = String(sanitized)
        return name.isEmpty ? UUID().uuidString : name
    }

    private nonisolated static func persistImportedBookData(_ data: Data, bookId: String, pathExtension: String) -> URL? {
        let fileURL = importedBooksDirectory()
            .appendingPathComponent(sanitizedStorageName(for: bookId))
            .appendingPathExtension(pathExtension)
        do {
            try data.write(to: fileURL, options: .atomic)
            return fileURL
        } catch {
            return nil
        }
    }

    private nonisolated static func removeManagedBookCopy(at fileURL: URL?) {
        guard let fileURL else { return }
        let managedDirectory = importedBooksDirectory().standardizedFileURL.path
        let targetPath = fileURL.standardizedFileURL.path
        guard targetPath.hasPrefix(managedDirectory) else { return }
        try? FileManager.default.removeItem(at: fileURL)
    }

    // MARK: - TXT Import (via Rust engine)

    /// Import a plain-text file using the Rust engine for parsing.
    private func importTxtBook(data: Data, fileURL: URL?) {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }

            // Parse via Rust engine for high-performance encoding detection
            let parseResult: TxtParseResult
            switch self.engine.parseTxt(data: data) {
            case .success(let result):
                parseResult = result
            case .failure(let error):
                DispatchQueue.main.async {
                    self.currentError = AppDisplayError(error)
                    self.endImportOperation()
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

            let storedFileURL = Self.persistImportedBookData(data, bookId: bookId, pathExtension: "txt")

            let book = LibraryBook(
                id: bookId,
                metadata: metadata,
                progress: 0.0,
                coverData: nil,
                fileURL: storedFileURL ?? fileURL,
                bookData: nil
            )

            // Cache parsed result for later reading
            TxtContentStore.shared.store(bookId: bookId, result: parseResult)

            DispatchQueue.main.async {
                if !self.libraryBooks.contains(where: { $0.id == book.id }) {
                    self.libraryBooks.append(book)
                }
                self.endImportOperation()
            }
        }
    }

    // MARK: - EPUB Import (via Rust engine)

    private func importEpubBook(data: Data, fileURL: URL?) {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }

            let openResult = self.engine.openBook(data: data)
            switch openResult {
            case .failure(let error):
                DispatchQueue.main.async {
                    self.currentError = AppDisplayError(error)
                    self.endImportOperation()
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
                let storedFileURL = Self.persistImportedBookData(data, bookId: metadata.id, pathExtension: "epub")
                let book = LibraryBook(
                    id: metadata.id,
                    metadata: metadata,
                    progress: 0.0,
                    coverData: coverData,
                    fileURL: storedFileURL ?? fileURL,
                    bookData: storedFileURL == nil ? data : nil
                )
                DispatchQueue.main.async {
                    if !self.libraryBooks.contains(where: { $0.id == book.id }) {
                        self.libraryBooks.append(book)
                    }
                    self.endImportOperation()
                }
            case .failure(let error):
                DispatchQueue.main.async {
                    self.currentError = AppDisplayError(error)
                    self.endImportOperation()
                }
            }
        }
    }

    /// Remove a book from the library.
    func removeBook(id: String) {
        let fileURL = libraryBooks.first(where: { $0.id == id })?.fileURL
        libraryBooks.removeAll { $0.id == id }
        TxtContentStore.shared.remove(bookId: id)
        chapterCache.invalidate(bookId: id)
        coverCache.remove(bookId: id)
        Self.removeManagedBookCopy(at: fileURL)
        if currentBookId == id {
            currentDestination = .library
            isSidebarVisible = true
        }
    }

    func releaseReaderResources(for bookId: String) {
        chapterCache.invalidate(bookId: bookId)
        guard let index = libraryBooks.firstIndex(where: { $0.id == bookId }),
              libraryBooks[index].fileURL != nil else { return }
        libraryBooks[index].bookData = nil
    }

    func exitReader(bookId: String? = nil) {
        if let targetBookId = bookId ?? currentBookId {
            releaseReaderResources(for: targetBookId)
        }
        currentDestination = .library
        isSidebarVisible = true
    }

    /// Open a book for reading.
    func openBook(_ book: LibraryBook) {
        if let currentBookId, currentBookId != book.id {
            releaseReaderResources(for: currentBookId)
        }
        currentDestination = .reader(bookId: book.id)
        isSidebarVisible = false
    }

    func setLibrarySidebarVisible(_ isVisible: Bool) {
        guard !isReaderActive else { return }
        isSidebarVisible = isVisible
    }

    func toggleSidebar() {
        if isReaderActive {
            NotificationCenter.default.post(name: .toggleTOC, object: nil)
        } else {
            isSidebarVisible.toggle()
        }
    }

    /// Reopen the EPUB file in the Rust engine for reading.
    /// Must be called before accessing chapter content, TOC, etc.
    func reopenEpubForReading(_ book: LibraryBook) -> Bool {
        // Try bookData first, then fall back to fileURL
        let data: Data?
        if let bd = book.bookData {
            data = bd
        } else if let url = book.fileURL {
            switch Self.readBookData(from: url) {
            case .success(let storedData):
                data = storedData
            case .failure:
                data = nil
            }
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
