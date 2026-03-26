//! Integration tests for the reader engine.
//!
//! End-to-end tests covering the full flow from EPUB/TXT parsing
//! through state management to storage persistence.

use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use core_parser::EpubParser;
use core_parser::TxtParser;
use core_state::StateManager;
use shared_types::{Annotation, Bookmark, NodeType};

/// Helper: create a minimal test EPUB in memory.
fn create_test_epub() -> Vec<u8> {
    let buf = Vec::new();
    let cursor = Cursor::new(buf);
    let mut zip = ZipWriter::new(cursor);
    let options = SimpleFileOptions::default();

    zip.start_file("mimetype", options).unwrap();
    zip.write_all(b"application/epub+zip").unwrap();

    zip.start_file("META-INF/container.xml", options).unwrap();
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
    )
    .unwrap();

    zip.start_file("OEBPS/content.opf", options).unwrap();
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Integration Test Book</dc:title>
    <dc:creator>Test Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="uid">integration-test-001</dc:identifier>
    <dc:date>2024-06-15</dc:date>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="chap01" href="chapter01.xhtml" media-type="application/xhtml+xml"/>
    <item id="chap02" href="chapter02.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chap01"/>
    <itemref idref="chap02"/>
  </spine>
</package>"#,
    )
    .unwrap();

    zip.start_file("OEBPS/nav.xhtml", options).unwrap();
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Navigation</title></head>
<body>
  <nav epub:type="toc">
    <ol>
      <li><a href="chapter01.xhtml">Chapter 1: Getting Started</a></li>
      <li><a href="chapter02.xhtml">Chapter 2: Advanced Topics</a></li>
    </ol>
  </nav>
</body>
</html>"#,
    )
    .unwrap();

    zip.start_file("OEBPS/chapter01.xhtml", options).unwrap();
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter 1</title></head>
<body>
  <h1>Chapter 1: Getting Started</h1>
  <p>Welcome to the integration test book. This is the first paragraph.</p>
  <p>This paragraph contains <em>emphasized</em> and <strong>bold</strong> text.</p>
  <blockquote>This is a blockquote for testing.</blockquote>
</body>
</html>"#,
    )
    .unwrap();

    zip.start_file("OEBPS/chapter02.xhtml", options).unwrap();
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter 2</title></head>
<body>
  <h1>Chapter 2: Advanced Topics</h1>
  <p>This chapter covers advanced topics.</p>
  <ul>
    <li>Item one</li>
    <li>Item two</li>
    <li>Item three</li>
  </ul>
  <p>End of chapter 2.</p>
</body>
</html>"#,
    )
    .unwrap();

    zip.finish().unwrap().into_inner()
}

#[test]
fn test_epub_full_pipeline() {
    let epub_data = create_test_epub();
    let mut parser = EpubParser::new(epub_data).unwrap();

    // Extract metadata
    let metadata = parser.parse_metadata().unwrap();
    assert_eq!(metadata.title, "Integration Test Book");
    assert_eq!(metadata.authors, vec!["Test Author"]);
    assert_eq!(metadata.language, Some("en".to_string()));
    assert!(metadata.file_hash.is_some());

    // Parse TOC
    let toc = parser.parse_toc().unwrap();
    assert_eq!(toc.len(), 2);
    assert_eq!(toc[0].title, "Chapter 1: Getting Started");
    assert_eq!(toc[1].title, "Chapter 2: Advanced Topics");

    // Get spine
    let spine = parser.get_spine().unwrap();
    assert_eq!(spine.len(), 2);

    // Parse each chapter
    for path in &spine {
        let nodes = parser.parse_chapter(path).unwrap();
        assert!(!nodes.is_empty());
        for node in &nodes {
            assert!(node.cfi_anchor.is_some());
        }
    }

    // Verify chapter 1 structure
    let ch1_nodes = parser.parse_chapter(&spine[0]).unwrap();
    assert_eq!(ch1_nodes[0].node_type, NodeType::Heading { level: 1 });
    assert_eq!(ch1_nodes[1].node_type, NodeType::Paragraph);
    assert_eq!(ch1_nodes[2].node_type, NodeType::Paragraph);
    assert_eq!(ch1_nodes[3].node_type, NodeType::BlockQuote);

    // Verify chapter 2 has a list
    let ch2_nodes = parser.parse_chapter(&spine[1]).unwrap();
    let has_list = ch2_nodes
        .iter()
        .any(|n| matches!(n.node_type, NodeType::List { .. }));
    assert!(has_list);
}

#[test]
fn test_txt_full_pipeline() {
    let content =
        "Chapter 1\n\nFirst paragraph.\n\nSecond paragraph.\n\nChapter 2\n\nMore content.";
    let data = content.as_bytes();

    let (nodes, encoding) = TxtParser::parse(data).unwrap();

    assert_eq!(encoding.encoding_name, "UTF-8");
    assert!(!encoding.had_replacements);
    assert_eq!(nodes.len(), 5);

    for (i, node) in nodes.iter().enumerate() {
        assert_eq!(node.node_type, NodeType::Paragraph);
        assert!(node.cfi_anchor.is_some());
        let expected_cfi = format!("/{}", (i as u32 + 1) * 2);
        assert_eq!(node.cfi_anchor.as_ref().unwrap(), &expected_cfi);
    }
}

#[test]
fn test_epub_parse_then_state_management() {
    let epub_data = create_test_epub();
    let mut parser = EpubParser::new(epub_data).unwrap();
    let metadata = parser.parse_metadata().unwrap();
    let spine = parser.get_spine().unwrap();
    let ch1_nodes = parser.parse_chapter(&spine[0]).unwrap();

    let sm = StateManager::new_in_memory("test-device").unwrap();

    // Register book
    sm.register_book(
        &metadata.id,
        &metadata.title,
        &metadata.authors.join(", "),
        &metadata.format.to_string(),
        metadata.file_hash.as_deref(),
        metadata.file_size,
    )
    .unwrap();

    // Update progress
    let first_cfi = ch1_nodes[0].cfi_anchor.as_ref().unwrap();
    sm.update_progress(&metadata.id, first_cfi, 10.0, 1000)
        .unwrap();

    let progress = sm.get_progress(&metadata.id).unwrap().unwrap();
    assert_eq!(progress.cfi_position, *first_cfi);
    assert!((progress.percentage - 10.0).abs() < f64::EPSILON);

    // Add bookmark
    let bm = Bookmark {
        id: "bm-int-1".to_string(),
        book_id: metadata.id.clone(),
        cfi_position: first_cfi.clone(),
        title: Some("Start of Chapter 1".to_string()),
        created_at: 1000,
    };
    sm.add_bookmark(&bm).unwrap();
    assert_eq!(sm.list_bookmarks(&metadata.id).unwrap().len(), 1);

    // Add annotation
    let ann = Annotation {
        id: "ann-int-1".to_string(),
        book_id: metadata.id.clone(),
        cfi_start: ch1_nodes[1].cfi_anchor.clone().unwrap(),
        cfi_end: ch1_nodes[2].cfi_anchor.clone().unwrap(),
        color_rgba: "#FFFF00FF".to_string(),
        note: Some("Important section".to_string()),
        created_at: 2000,
    };
    sm.add_annotation(&ann).unwrap();
    assert_eq!(sm.list_annotations(&metadata.id).unwrap().len(), 1);

    // Set preferences
    sm.set_preference("font_size", "16", 3000).unwrap();
    sm.set_preference("theme", "dark", 3001).unwrap();
    assert_eq!(sm.get_preference("font_size").unwrap().unwrap().value, "16");

    // Verify op_log
    let ops = sm.database().get_unsynced_ops().unwrap();
    assert_eq!(ops.len(), 5); // progress + bookmark + annotation + 2 prefs

    // Verify ordering
    for i in 1..ops.len() {
        assert!(ops[i].3 >= ops[i - 1].3);
    }
}

#[test]
fn test_multiple_books_state_isolation() {
    let sm = StateManager::new_in_memory("test-device").unwrap();

    sm.register_book("book-a", "Book A", "Author A", "epub", None, None)
        .unwrap();
    sm.register_book("book-b", "Book B", "Author B", "txt", None, None)
        .unwrap();

    let bm_a = Bookmark {
        id: "bm-a".to_string(),
        book_id: "book-a".to_string(),
        cfi_position: "/2".to_string(),
        title: None,
        created_at: 1000,
    };
    let bm_b = Bookmark {
        id: "bm-b".to_string(),
        book_id: "book-b".to_string(),
        cfi_position: "/4".to_string(),
        title: None,
        created_at: 2000,
    };

    sm.add_bookmark(&bm_a).unwrap();
    sm.add_bookmark(&bm_b).unwrap();

    assert_eq!(sm.list_bookmarks("book-a").unwrap().len(), 1);
    assert_eq!(sm.list_bookmarks("book-b").unwrap().len(), 1);
    assert_eq!(sm.list_bookmarks("book-a").unwrap()[0].id, "bm-a");
    assert_eq!(sm.list_bookmarks("book-b").unwrap()[0].id, "bm-b");

    sm.update_progress("book-a", "/2", 50.0, 3000).unwrap();
    sm.update_progress("book-b", "/4", 25.0, 4000).unwrap();

    let pa = sm.get_progress("book-a").unwrap().unwrap();
    let pb = sm.get_progress("book-b").unwrap().unwrap();
    assert!((pa.percentage - 50.0).abs() < f64::EPSILON);
    assert!((pb.percentage - 25.0).abs() < f64::EPSILON);
}
