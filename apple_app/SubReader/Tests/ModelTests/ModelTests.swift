// ModelTests — Unit tests for ReaderModels JSON serialization.

import XCTest
@testable import ReaderModels

final class ModelTests: XCTestCase {

    // MARK: - BookMetadata

    func testBookMetadataDecoding() throws {
        let json = """
        {
            "id": "test-id",
            "title": "Test Book",
            "authors": ["Author One"],
            "language": "en",
            "publish_date": null,
            "cover_image_ref": null,
            "format": "Epub",
            "file_hash": null,
            "file_size": 1024
        }
        """
        let data = json.data(using: .utf8)!
        let meta = try JSONCoding.decoder.decode(BookMetadata.self, from: data)
        XCTAssertEqual(meta.id, "test-id")
        XCTAssertEqual(meta.title, "Test Book")
        XCTAssertEqual(meta.authors, ["Author One"])
        XCTAssertEqual(meta.format, .epub)
        XCTAssertEqual(meta.fileSize, 1024)
    }

    // MARK: - NodeType

    func testNodeTypeSimpleDecoding() throws {
        let json = "\"Paragraph\""
        let data = json.data(using: .utf8)!
        let nodeType = try JSONCoding.decoder.decode(NodeType.self, from: data)
        XCTAssertEqual(nodeType, .paragraph)
    }

    func testNodeTypeHeadingDecoding() throws {
        let json = "{\"Heading\":{\"level\":2}}"
        let data = json.data(using: .utf8)!
        let nodeType = try JSONCoding.decoder.decode(NodeType.self, from: data)
        XCTAssertEqual(nodeType, .heading(level: 2))
    }

    func testNodeTypeListDecoding() throws {
        let json = "{\"List\":{\"ordered\":true}}"
        let data = json.data(using: .utf8)!
        let nodeType = try JSONCoding.decoder.decode(NodeType.self, from: data)
        XCTAssertEqual(nodeType, .list(ordered: true))
    }

    func testNodeTypeRoundTrip() throws {
        let types: [NodeType] = [
            .document, .heading(level: 1), .heading(level: 6),
            .paragraph, .text, .image, .link,
            .list(ordered: true), .list(ordered: false),
            .listItem, .emphasis, .strong, .code,
            .blockQuote, .table, .tableRow, .tableCell,
            .lineBreak, .span
        ]

        for nodeType in types {
            let encoded = try JSONCoding.encoder.encode(nodeType)
            let decoded = try JSONCoding.decoder.decode(NodeType.self, from: encoded)
            XCTAssertEqual(nodeType, decoded, "Round-trip failed for \(nodeType)")
        }
    }

    // MARK: - DomNode

    func testDomNodeDecoding() throws {
        let json = """
        {
            "node_type": "Paragraph",
            "cfi_anchor": "/6/4!/4/2",
            "text": null,
            "attributes": [],
            "children": [
                {
                    "node_type": "Text",
                    "cfi_anchor": null,
                    "text": "Hello, world!",
                    "attributes": [],
                    "children": []
                }
            ]
        }
        """
        let data = json.data(using: .utf8)!
        let node = try JSONCoding.decoder.decode(DomNode.self, from: data)
        XCTAssertEqual(node.nodeType, .paragraph)
        XCTAssertEqual(node.children.count, 1)
        XCTAssertEqual(node.children.first?.text, "Hello, world!")
    }

    // MARK: - ReadingProgress

    func testReadingProgressDecoding() throws {
        let json = """
        {
            "book_id": "book-1",
            "cfi_position": "/6/4!/4/2:0",
            "percentage": 42.5,
            "hlc_timestamp": 1000
        }
        """
        let data = json.data(using: .utf8)!
        let progress = try JSONCoding.decoder.decode(ReadingProgress.self, from: data)
        XCTAssertEqual(progress.bookId, "book-1")
        XCTAssertEqual(progress.percentage, 42.5)
    }

    // MARK: - Bookmark

    func testBookmarkDecoding() throws {
        let json = """
        {
            "id": "bm-1",
            "book_id": "book-1",
            "cfi_position": "/6/4!/4/2:0",
            "title": "Chapter 1",
            "created_at": 1000
        }
        """
        let data = json.data(using: .utf8)!
        let bookmark = try JSONCoding.decoder.decode(Bookmark.self, from: data)
        XCTAssertEqual(bookmark.id, "bm-1")
        XCTAssertEqual(bookmark.title, "Chapter 1")
    }

    // MARK: - Annotation

    func testAnnotationDecoding() throws {
        let json = """
        {
            "id": "ann-1",
            "book_id": "book-1",
            "cfi_start": "/6/4!/4/2:0",
            "cfi_end": "/6/4!/4/2:10",
            "color_rgba": "#FF0000FF",
            "note": "Important",
            "created_at": 1000
        }
        """
        let data = json.data(using: .utf8)!
        let annotation = try JSONCoding.decoder.decode(Annotation.self, from: data)
        XCTAssertEqual(annotation.id, "ann-1")
        XCTAssertEqual(annotation.colorRgba, "#FF0000FF")
        XCTAssertEqual(annotation.note, "Important")
    }

    // MARK: - TocEntry

    func testTocEntryDecoding() throws {
        let json = """
        {
            "title": "Chapter 1",
            "href": "chapter1.xhtml",
            "level": 0,
            "children": [
                {
                    "title": "Section 1.1",
                    "href": "chapter1.xhtml#s1",
                    "level": 1,
                    "children": []
                }
            ]
        }
        """
        let data = json.data(using: .utf8)!
        let toc = try JSONCoding.decoder.decode(TocEntry.self, from: data)
        XCTAssertEqual(toc.title, "Chapter 1")
        XCTAssertEqual(toc.children.count, 1)
        XCTAssertEqual(toc.children.first?.title, "Section 1.1")
    }
}
