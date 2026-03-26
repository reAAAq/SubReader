// TxtContentStore — In-memory store for parsed plain-text book content.
//
// Splits TXT content into chapters by common patterns (blank-line separated blocks,
// "Chapter N" headings, etc.) and provides chapter-based access.

import Foundation

/// Stores parsed TXT content keyed by book ID.
/// Thread-safe via a serial dispatch queue.
final class TxtContentStore: @unchecked Sendable {

    static let shared = TxtContentStore()

    private let queue = DispatchQueue(label: "com.subreader.txt-store", attributes: .concurrent)
    private var storage: [String: TxtBook] = [:]

    private init() {}

    /// A parsed TXT book with chapters.
    struct TxtBook {
        let fullContent: String
        let chapters: [TxtChapter]
    }

    /// A single chapter in a TXT book.
    struct TxtChapter {
        let title: String
        let content: String
    }

    // MARK: - Public API

    /// Store and parse TXT content for a book.
    func store(bookId: String, content: String) {
        let chapters = Self.splitIntoChapters(content)
        let book = TxtBook(fullContent: content, chapters: chapters)
        queue.async(flags: .barrier) {
            self.storage[bookId] = book
        }
    }

    /// Retrieve the parsed TXT book.
    func get(bookId: String) -> TxtBook? {
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

    /// Reload content from file URL if not in memory.
    func loadIfNeeded(bookId: String, fileURL: URL?) -> TxtBook? {
        if let existing = get(bookId: bookId) {
            return existing
        }
        guard let url = fileURL,
              let data = try? Data(contentsOf: url) else { return nil }

        let content: String
        if let utf8 = String(data: data, encoding: .utf8) {
            content = utf8
        } else if let gb = String(data: data, encoding: .init(rawValue: CFStringConvertEncodingToNSStringEncoding(CFStringEncoding(CFStringEncodings.GB_18030_2000.rawValue)))) {
            content = gb
        } else if let latin = String(data: data, encoding: .isoLatin1) {
            content = latin
        } else {
            return nil
        }

        store(bookId: bookId, content: content)
        return get(bookId: bookId)
    }

    // MARK: - Chapter Splitting

    /// Split plain text into chapters using common patterns.
    private static func splitIntoChapters(_ text: String) -> [TxtChapter] {
        // Try to detect chapter headings with common patterns
        let chapterPattern = #"(?m)^[\s]*(?:第[一二三四五六七八九十百千零\d]+[章节回卷篇]|Chapter\s+\d+|CHAPTER\s+\d+)[\s:：.、]*(.*)$"#

        guard let regex = try? NSRegularExpression(pattern: chapterPattern, options: []) else {
            return splitBySize(text)
        }

        let nsText = text as NSString
        let matches = regex.matches(in: text, options: [], range: NSRange(location: 0, length: nsText.length))

        if matches.count >= 2 {
            // Found chapter headings — split by them
            var chapters: [TxtChapter] = []

            // Content before the first chapter heading
            let preContent = nsText.substring(with: NSRange(location: 0, length: matches[0].range.location)).trimmingCharacters(in: .whitespacesAndNewlines)
            if !preContent.isEmpty {
                chapters.append(TxtChapter(title: "Preface", content: preContent))
            }

            for (i, match) in matches.enumerated() {
                let title = nsText.substring(with: match.range).trimmingCharacters(in: .whitespacesAndNewlines)
                let contentStart = match.range.location + match.range.length
                let contentEnd: Int
                if i + 1 < matches.count {
                    contentEnd = matches[i + 1].range.location
                } else {
                    contentEnd = nsText.length
                }
                let content = nsText.substring(with: NSRange(location: contentStart, length: contentEnd - contentStart)).trimmingCharacters(in: .whitespacesAndNewlines)
                chapters.append(TxtChapter(title: title, content: content))
            }

            return chapters
        }

        // No chapter headings found — split by size
        return splitBySize(text)
    }

    /// Split text into roughly equal-sized chunks (~5000 chars each).
    private static func splitBySize(_ text: String, chunkSize: Int = 5000) -> [TxtChapter] {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return [] }

        // For short texts, return as single chapter
        if trimmed.count <= chunkSize {
            return [TxtChapter(title: "Full Text", content: trimmed)]
        }

        var chapters: [TxtChapter] = []
        var startIndex = trimmed.startIndex
        var chapterNum = 1

        while startIndex < trimmed.endIndex {
            // Find a good break point near chunkSize
            var endIndex = trimmed.index(startIndex, offsetBy: chunkSize, limitedBy: trimmed.endIndex) ?? trimmed.endIndex

            if endIndex < trimmed.endIndex {
                // Try to break at a paragraph boundary (double newline)
                let searchRange = trimmed.index(endIndex, offsetBy: -200, limitedBy: startIndex) ?? startIndex
                if let paraBreak = trimmed.range(of: "\n\n", options: .backwards, range: searchRange..<endIndex) {
                    endIndex = paraBreak.upperBound
                } else if let lineBreak = trimmed.range(of: "\n", options: .backwards, range: searchRange..<endIndex) {
                    // Fall back to single newline
                    endIndex = lineBreak.upperBound
                }
            }

            let chunk = String(trimmed[startIndex..<endIndex]).trimmingCharacters(in: .whitespacesAndNewlines)
            if !chunk.isEmpty {
                chapters.append(TxtChapter(title: "Section \(chapterNum)", content: chunk))
                chapterNum += 1
            }
            startIndex = endIndex
        }

        return chapters
    }
}
