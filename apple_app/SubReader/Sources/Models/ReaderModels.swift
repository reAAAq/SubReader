// ReaderModels — Shared data models for SubReader.
//
// This module contains pure Codable structs that map 1:1 to Rust shared_types.
// Zero external dependencies. All types are Sendable for safe cross-thread use.

import Foundation

// MARK: - Book Metadata

/// Metadata extracted from a book file (EPUB, TXT, etc.).
public struct BookMetadata: Codable, Sendable, Hashable, Identifiable {
    public let id: String
    public let title: String
    public let authors: [String]
    public let language: String?
    public let publishDate: String?
    public let coverImageRef: String?
    public let format: BookFormat
    public let fileHash: String?
    public let fileSize: UInt64?

    public init(
        id: String,
        title: String,
        authors: [String],
        language: String? = nil,
        publishDate: String? = nil,
        coverImageRef: String? = nil,
        format: BookFormat,
        fileHash: String? = nil,
        fileSize: UInt64? = nil
    ) {
        self.id = id
        self.title = title
        self.authors = authors
        self.language = language
        self.publishDate = publishDate
        self.coverImageRef = coverImageRef
        self.format = format
        self.fileHash = fileHash
        self.fileSize = fileSize
    }

    enum CodingKeys: String, CodingKey {
        case id, title, authors, language
        case publishDate = "publish_date"
        case coverImageRef = "cover_image_ref"
        case format
        case fileHash = "file_hash"
        case fileSize = "file_size"
    }
}

/// Supported book formats.
public enum BookFormat: String, Codable, Sendable, Hashable {
    case epub = "Epub"
    case txt = "Txt"
}

// MARK: - Table of Contents

/// A single entry in the table of contents (recursive tree).
public struct TocEntry: Codable, Sendable, Hashable, Identifiable {
    public var id: String { "\(href)_\(level)_\(title)" }
    public let title: String
    public let href: String
    public let level: UInt32
    public let children: [TocEntry]

    public init(title: String, href: String, level: UInt32, children: [TocEntry] = []) {
        self.title = title
        self.href = href
        self.level = level
        self.children = children
    }
}

// MARK: - DOM Tree

/// A node in the platform-independent DOM tree.
public struct DomNode: Codable, Sendable, Hashable, Identifiable {
    public var id: String { cfiAnchor ?? UUID().uuidString }
    public let nodeType: NodeType
    public let cfiAnchor: String?
    public let text: String?
    public let attributes: [[String]]
    public let children: [DomNode]

    public init(
        nodeType: NodeType,
        cfiAnchor: String? = nil,
        text: String? = nil,
        attributes: [[String]] = [],
        children: [DomNode] = []
    ) {
        self.nodeType = nodeType
        self.cfiAnchor = cfiAnchor
        self.text = text
        self.attributes = attributes
        self.children = children
    }

    enum CodingKeys: String, CodingKey {
        case nodeType = "node_type"
        case cfiAnchor = "cfi_anchor"
        case text, attributes, children
    }
}

/// Types of DOM nodes supported by the parser.
/// Matches Rust `NodeType` enum serialization format exactly.
public enum NodeType: Codable, Sendable, Hashable {
    case document
    case heading(level: UInt8)
    case paragraph
    case text
    case image
    case link
    case list(ordered: Bool)
    case listItem
    case emphasis
    case strong
    case code
    case blockQuote
    case table
    case tableRow
    case tableCell
    case lineBreak
    case span

    // Custom Codable to match Rust's serde serialization format
    enum CodingKeys: String, CodingKey {
        case document = "Document"
        case heading = "Heading"
        case paragraph = "Paragraph"
        case text = "Text"
        case image = "Image"
        case link = "Link"
        case list = "List"
        case listItem = "ListItem"
        case emphasis = "Emphasis"
        case strong = "Strong"
        case code = "Code"
        case blockQuote = "BlockQuote"
        case table = "Table"
        case tableRow = "TableRow"
        case tableCell = "TableCell"
        case lineBreak = "LineBreak"
        case span = "Span"
    }

    enum HeadingKeys: String, CodingKey {
        case level
    }

    enum ListKeys: String, CodingKey {
        case ordered
    }

    public init(from decoder: Decoder) throws {
        // Try as simple string first (for variants without associated values)
        if let container = try? decoder.singleValueContainer(),
           let stringValue = try? container.decode(String.self) {
            switch stringValue {
            case "Document": self = .document
            case "Paragraph": self = .paragraph
            case "Text": self = .text
            case "Image": self = .image
            case "Link": self = .link
            case "ListItem": self = .listItem
            case "Emphasis": self = .emphasis
            case "Strong": self = .strong
            case "Code": self = .code
            case "BlockQuote": self = .blockQuote
            case "Table": self = .table
            case "TableRow": self = .tableRow
            case "TableCell": self = .tableCell
            case "LineBreak": self = .lineBreak
            case "Span": self = .span
            default:
                throw DecodingError.dataCorrupted(
                    .init(codingPath: decoder.codingPath, debugDescription: "Unknown NodeType: \(stringValue)")
                )
            }
            return
        }

        // Try as object (for Heading { level } and List { ordered })
        let container = try decoder.container(keyedBy: CodingKeys.self)

        if let headingContainer = try? container.nestedContainer(keyedBy: HeadingKeys.self, forKey: .heading) {
            let level = try headingContainer.decode(UInt8.self, forKey: .level)
            self = .heading(level: level)
        } else if let listContainer = try? container.nestedContainer(keyedBy: ListKeys.self, forKey: .list) {
            let ordered = try listContainer.decode(Bool.self, forKey: .ordered)
            self = .list(ordered: ordered)
        } else {
            throw DecodingError.dataCorrupted(
                .init(codingPath: decoder.codingPath, debugDescription: "Unknown NodeType object")
            )
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .heading(let level):
            var container = encoder.container(keyedBy: CodingKeys.self)
            var nested = container.nestedContainer(keyedBy: HeadingKeys.self, forKey: .heading)
            try nested.encode(level, forKey: .level)
        case .list(let ordered):
            var container = encoder.container(keyedBy: CodingKeys.self)
            var nested = container.nestedContainer(keyedBy: ListKeys.self, forKey: .list)
            try nested.encode(ordered, forKey: .ordered)
        default:
            var container = encoder.singleValueContainer()
            switch self {
            case .document: try container.encode("Document")
            case .paragraph: try container.encode("Paragraph")
            case .text: try container.encode("Text")
            case .image: try container.encode("Image")
            case .link: try container.encode("Link")
            case .listItem: try container.encode("ListItem")
            case .emphasis: try container.encode("Emphasis")
            case .strong: try container.encode("Strong")
            case .code: try container.encode("Code")
            case .blockQuote: try container.encode("BlockQuote")
            case .table: try container.encode("Table")
            case .tableRow: try container.encode("TableRow")
            case .tableCell: try container.encode("TableCell")
            case .lineBreak: try container.encode("LineBreak")
            case .span: try container.encode("Span")
            default: break
            }
        }
    }
}

// MARK: - Reading Progress

/// Reading progress for a specific book.
public struct ReadingProgress: Codable, Sendable, Hashable {
    public let bookId: String
    public let cfiPosition: String
    public let percentage: Double
    public let hlcTimestamp: UInt64

    public init(bookId: String, cfiPosition: String, percentage: Double, hlcTimestamp: UInt64) {
        self.bookId = bookId
        self.cfiPosition = cfiPosition
        self.percentage = percentage
        self.hlcTimestamp = hlcTimestamp
    }

    enum CodingKeys: String, CodingKey {
        case bookId = "book_id"
        case cfiPosition = "cfi_position"
        case percentage
        case hlcTimestamp = "hlc_timestamp"
    }
}

// MARK: - Bookmark

/// A bookmark placed by the user.
public struct Bookmark: Codable, Sendable, Hashable, Identifiable {
    public let id: String
    public let bookId: String
    public let cfiPosition: String
    public let title: String?
    public let createdAt: UInt64

    public init(id: String, bookId: String, cfiPosition: String, title: String? = nil, createdAt: UInt64) {
        self.id = id
        self.bookId = bookId
        self.cfiPosition = cfiPosition
        self.title = title
        self.createdAt = createdAt
    }

    enum CodingKeys: String, CodingKey {
        case id
        case bookId = "book_id"
        case cfiPosition = "cfi_position"
        case title
        case createdAt = "created_at"
    }
}

// MARK: - Annotation

/// A highlight/annotation created by the user.
public struct Annotation: Codable, Sendable, Hashable, Identifiable {
    public let id: String
    public let bookId: String
    public let cfiStart: String
    public let cfiEnd: String
    public let colorRgba: String
    public let note: String?
    public let createdAt: UInt64

    public init(
        id: String,
        bookId: String,
        cfiStart: String,
        cfiEnd: String,
        colorRgba: String,
        note: String? = nil,
        createdAt: UInt64
    ) {
        self.id = id
        self.bookId = bookId
        self.cfiStart = cfiStart
        self.cfiEnd = cfiEnd
        self.colorRgba = colorRgba
        self.note = note
        self.createdAt = createdAt
    }

    enum CodingKeys: String, CodingKey {
        case id
        case bookId = "book_id"
        case cfiStart = "cfi_start"
        case cfiEnd = "cfi_end"
        case colorRgba = "color_rgba"
        case note
        case createdAt = "created_at"
    }
}

// MARK: - Shared JSON Decoder

/// Shared, pre-configured JSONDecoder for high-performance decoding.
/// Reuse this instance to avoid repeated allocator overhead.
public enum JSONCoding {
    /// Shared decoder configured for Rust serde JSON output.
    public static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        return decoder
    }()

    /// Shared encoder configured to match Rust serde JSON format.
    public static let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        return encoder
    }()
}
