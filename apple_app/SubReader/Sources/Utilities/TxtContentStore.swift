// TxtContentStore — In-memory store for parsed plain-text book content.
//
// Delegates TXT parsing to the Rust engine for high-performance encoding
// detection and chapter splitting. Caches results keyed by book ID.

import Foundation
import ReaderModels
import ReaderBridge

/// Stores parsed TXT content keyed by book ID.
/// Thread-safe via a serial dispatch queue.
final class TxtContentStore: @unchecked Sendable {

    static let shared = TxtContentStore()

    private let queue = DispatchQueue(label: "com.subreader.txt-store", attributes: .concurrent)
    private var storage: [String: TxtParseResult] = [:]

    private init() {}

    // MARK: - Public API

    /// Store a pre-parsed TXT result for a book.
    func store(bookId: String, result: TxtParseResult) {
        queue.async(flags: .barrier) {
            self.storage[bookId] = result
        }
    }

    /// Retrieve the parsed TXT result.
    func get(bookId: String) -> TxtParseResult? {
        queue.sync {
            storage[bookId]
        }
    }

    /// Remove stored content for a book.
    func remove(bookId: String) {
        queue.async(flags: .barrier) {
            self.storage.removeValue(forKey: bookId)
        }
    }

    /// Load and parse TXT content from file URL using the Rust engine.
    /// Returns cached result if available, otherwise reads file and parses via Rust.
    func loadIfNeeded(bookId: String, fileURL: URL?, engine: ReaderEngineProtocol) -> TxtParseResult? {
        if let existing = get(bookId: bookId) {
            return existing
        }
        guard let url = fileURL,
              let data = try? Data(contentsOf: url) else { return nil }

        switch engine.parseTxt(data: data) {
        case .success(let result):
            store(bookId: bookId, result: result)
            return result
        case .failure:
            return nil
        }
    }
}
