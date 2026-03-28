// DIContainer — Lightweight dependency injection container.
//
// Manages singleton lifecycles for engine, caches, and services.

import Foundation
import ReaderModels

/// Dependency injection container for the SubReader app.
@MainActor
public final class DIContainer: ObservableObject {

    /// The reader engine instance (protocol-typed for testability).
    public let engine: any ReaderEngineProtocol

    /// Chapter content cache (keyed by book and chapter identity).
    public let chapterCache = ChapterCache()

    /// Cover image cache (memory + disk).
    public let coverCache = CoverImageCache()

    /// Shared instance for the app.
    public static let shared = DIContainer()

    /// Initialize with a specific engine (for testing).
    public init(engine: any ReaderEngineProtocol = RustCore()) {
        self.engine = engine
    }
}

// MARK: - Chapter Cache

public struct ChapterCacheKey: Hashable, Sendable {
    public let bookId: String
    public let chapterPath: String
    public let themeHash: String

    public init(bookId: String, chapterPath: String, themeHash: String) {
        self.bookId = bookId
        self.chapterPath = chapterPath
        self.themeHash = themeHash
    }

    fileprivate var storageKey: String {
        [bookId, themeHash, chapterPath].joined(separator: "::")
    }
}

/// LRU cache for rendered chapter content.
/// Keyed by (bookId, chapterPath, themeHash) to isolate books and themes.
public final class ChapterCache: @unchecked Sendable {
    private let cache = NSCache<NSString, CacheEntry>()
    private let keyIndexQueue = DispatchQueue(label: "SubReader.ChapterCache.Index", attributes: .concurrent)
    private var keysByBookID: [String: Set<String>] = [:]

    public init() {
        cache.countLimit = 5 // Keep up to 5 chapters in memory
    }

    public func get(key: ChapterCacheKey) -> Any? {
        let storageKey = key.storageKey
        guard let entry = cache.object(forKey: storageKey as NSString) else {
            return nil
        }
        guard entry.bookId == key.bookId else {
            removeStorageKey(storageKey, for: entry.bookId)
            cache.removeObject(forKey: storageKey as NSString)
            return nil
        }
        return entry.value
    }

    public func set(key: ChapterCacheKey, value: Any) {
        let storageKey = key.storageKey
        cache.setObject(CacheEntry(bookId: key.bookId, value: value), forKey: storageKey as NSString)
        keyIndexQueue.sync(flags: .barrier) {
            var keys = keysByBookID[key.bookId, default: []]
            keys.insert(storageKey)
            keysByBookID[key.bookId] = keys
        }
    }

    public func invalidate(bookId: String) {
        keyIndexQueue.sync(flags: .barrier) {
            let storageKeys = keysByBookID.removeValue(forKey: bookId) ?? []
            for storageKey in storageKeys {
                cache.removeObject(forKey: storageKey as NSString)
            }
        }
    }

    public func invalidate() {
        keyIndexQueue.sync(flags: .barrier) {
            keysByBookID.removeAll()
            cache.removeAllObjects()
        }
    }

    private func removeStorageKey(_ storageKey: String, for bookId: String) {
        keyIndexQueue.sync(flags: .barrier) {
            guard var keys = keysByBookID[bookId] else { return }
            keys.remove(storageKey)
            if keys.isEmpty {
                keysByBookID.removeValue(forKey: bookId)
            } else {
                keysByBookID[bookId] = keys
            }
        }
    }

    private class CacheEntry {
        let bookId: String
        let value: Any

        init(bookId: String, value: Any) {
            self.bookId = bookId
            self.value = value
        }
    }
}

// MARK: - Cover Image Cache

/// Two-tier cache for book cover images: NSCache (memory) + disk.
public final class CoverImageCache: @unchecked Sendable {
    private let memoryCache = NSCache<NSString, NSData>()
    private let cacheDir: URL

    public init() {
        let cachesDir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first!
        cacheDir = cachesDir.appendingPathComponent("SubReader/covers", isDirectory: true)
        try? FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true)
        memoryCache.countLimit = 50
    }

    public func get(bookId: String) -> Data? {
        let key = bookId as NSString
        // Check memory first
        if let data = memoryCache.object(forKey: key) {
            return data as Data
        }
        // Check disk
        let fileURL = cacheDir.appendingPathComponent("\(bookId).jpg")
        if let data = try? Data(contentsOf: fileURL) {
            memoryCache.setObject(data as NSData, forKey: key)
            return data
        }
        return nil
    }

    public func set(bookId: String, data: Data) {
        let key = bookId as NSString
        memoryCache.setObject(data as NSData, forKey: key)
        // Write to disk asynchronously
        let fileURL = cacheDir.appendingPathComponent("\(bookId).jpg")
        DispatchQueue.global(qos: .utility).async {
            try? data.write(to: fileURL)
        }
    }

    public func remove(bookId: String) {
        let key = bookId as NSString
        memoryCache.removeObject(forKey: key)
        let fileURL = cacheDir.appendingPathComponent("\(bookId).jpg")
        try? FileManager.default.removeItem(at: fileURL)
    }
}
