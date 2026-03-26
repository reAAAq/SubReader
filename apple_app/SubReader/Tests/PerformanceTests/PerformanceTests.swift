// PerformanceTests — Performance benchmarks for FFI and rendering.

import XCTest
@testable import ReaderModels

final class PerformanceTests: XCTestCase {

    // MARK: - JSON Decoding Performance

    func testNodeTypeDecodingPerformance() throws {
        // Generate a large JSON array of DomNodes
        let sampleNode = """
        {"node_type":"Paragraph","cfi_anchor":"/6/4!/4/2","text":null,"attributes":[],"children":[{"node_type":"Text","cfi_anchor":null,"text":"Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.","attributes":[],"children":[]}]}
        """
        let jsonArray = "[" + Array(repeating: sampleNode, count: 100).joined(separator: ",") + "]"
        let data = jsonArray.data(using: .utf8)!

        measure {
            for _ in 0..<10 {
                let _ = try? JSONCoding.decoder.decode([DomNode].self, from: data)
            }
        }
    }

    func testBookMetadataDecodingPerformance() throws {
        let json = """
        {"id":"test-id","title":"Test Book","authors":["Author One","Author Two"],"language":"en","publish_date":"2024-01-01","cover_image_ref":"cover.jpg","format":"Epub","file_hash":"abc123","file_size":1048576}
        """
        let data = json.data(using: .utf8)!

        measure {
            for _ in 0..<1000 {
                let _ = try? JSONCoding.decoder.decode(BookMetadata.self, from: data)
            }
        }
    }

    func testLargeChapterDecoding() throws {
        // Simulate a large chapter with 500 paragraphs
        var nodes: [String] = []
        for i in 0..<500 {
            nodes.append("""
            {"node_type":"Paragraph","cfi_anchor":"/6/4!/4/\(i*2)","text":null,"attributes":[],"children":[{"node_type":"Text","cfi_anchor":null,"text":"Paragraph \(i): Lorem ipsum dolor sit amet, consectetur adipiscing elit. Vestibulum ante ipsum primis in faucibus orci luctus et ultrices posuere cubilia curae.","attributes":[],"children":[]}]}
            """)
        }
        let jsonArray = "[" + nodes.joined(separator: ",") + "]"
        let data = jsonArray.data(using: .utf8)!

        // Target: < 3ms for 100KB chapter
        measure {
            let _ = try? JSONCoding.decoder.decode([DomNode].self, from: data)
        }
    }
}
