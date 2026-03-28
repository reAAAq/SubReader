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
    private let maxCachedBooks = 4

    private let queue = DispatchQueue(label: "com.subreader.txt-store", attributes: .concurrent)
    private var storage: [String: TxtParseResult] = [:]
    private var accessOrder: [String] = []

    private init() {}

    // MARK: - Public API

    /// Store a pre-parsed TXT result for a book.
    func store(bookId: String, result: TxtParseResult) {
        queue.sync(flags: .barrier) {
            self.storage[bookId] = result
            self.touchLocked(bookId: bookId)
            self.trimToLimitLocked()
        }
    }

    /// Retrieve the parsed TXT result.
    func get(bookId: String) -> TxtParseResult? {
        let result = queue.sync {
            storage[bookId]
        }
        guard let result else { return nil }
        queue.async(flags: .barrier) {
            self.touchLocked(bookId: bookId)
        }
        return result
    }

    /// Remove stored content for a book.
    func remove(bookId: String) {
        queue.sync(flags: .barrier) {
            self.storage.removeValue(forKey: bookId)
            self.accessOrder.removeAll { $0 == bookId }
        }
    }

    func clear() {
        queue.sync(flags: .barrier) {
            self.storage.removeAll()
            self.accessOrder.removeAll()
        }
    }

    /// Load and parse TXT content from file URL using the Rust engine.
    /// Returns cached result if available, otherwise reads file and parses via Rust.
    func loadIfNeeded(bookId: String, fileURL: URL?, engine: ReaderEngineProtocol) -> TxtParseResult? {
        if let existing = get(bookId: bookId) {
            return existing
        }
        guard let url = fileURL,
              let data = try? Data(contentsOf: url, options: [.mappedIfSafe]) else { return nil }

        switch engine.parseTxt(data: data) {
        case .success(let result):
            store(bookId: bookId, result: result)
            return result
        case .failure:
            return nil
        }
    }

    private func touchLocked(bookId: String) {
        accessOrder.removeAll { $0 == bookId }
        accessOrder.append(bookId)
    }

    private func trimToLimitLocked() {
        while storage.count > maxCachedBooks, let evictedBookId = accessOrder.first {
            accessOrder.removeFirst()
            storage.removeValue(forKey: evictedBookId)
        }
    }
}
