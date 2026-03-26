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

    /// Chapter content cache (keyed by chapter path).
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

/// LRU cache for rendered chapter content.
/// Keyed by (chapterPath, themeHash) to invalidate on theme changes.
public final class ChapterCache: @unchecked Sendable {
    private let cache = NSCache<NSString, CacheEntry>()

    public init() {
        cache.countLimit = 5 // Keep up to 5 chapters in memory
    }

    public func get(key: String) -> Any? {
        cache.object(forKey: key as NSString)?.value
    }

    public func set(key: String, value: Any) {
        cache.setObject(CacheEntry(value: value), forKey: key as NSString)
    }

    public func invalidate() {
        cache.removeAllObjects()
    }

    private class CacheEntry {
        let value: Any
        init(value: Any) { self.value = value }
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
}
