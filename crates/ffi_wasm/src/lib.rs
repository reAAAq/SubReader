//! WASM bridge layer for VS Code plugin and web environments.
//!
//! Exports functions via `wasm-bindgen` for use in JavaScript/TypeScript.
//! Storage operations are bridged through JavaScript imports since SQLite
//! is not available in WASM environments.
//!
//! This module provides parser-equivalent functionality to `ffi_c`,
//! with state management delegated to the JS host.

use std::cell::RefCell;

use wasm_bindgen::prelude::*;

use core_parser::epub::{parse_xhtml_to_dom, EpubParser};
use core_parser::txt::TxtParser;

// ─── Thread-local state for WASM (single-threaded) ───────────────────────────

thread_local! {
    static CURRENT_PARSER: RefCell<Option<EpubParser>> = const { RefCell::new(None) };
    static RETURN_BUFFER: RefCell<String> = const { RefCell::new(String::new()) };
}

// ─── Engine Lifecycle ────────────────────────────────────────────────────────

/// Initialize the WASM reader engine.
#[wasm_bindgen]
pub fn wasm_engine_init() -> bool {
    true
}

/// Get the engine version string.
#[wasm_bindgen]
pub fn wasm_engine_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Destroy the WASM reader engine and free resources.
#[wasm_bindgen]
pub fn wasm_engine_destroy() {
    CURRENT_PARSER.with(|p| {
        *p.borrow_mut() = None;
    });
}

// ─── EPUB Operations ─────────────────────────────────────────────────────────

/// Open an EPUB book from raw bytes.
/// Returns true on success, false on failure.
#[wasm_bindgen]
pub fn wasm_open_epub(data: &[u8]) -> Result<bool, JsError> {
    let parser =
        EpubParser::new(data.to_vec()).map_err(|e| JsError::new(&format!("Parse error: {e}")))?;

    CURRENT_PARSER.with(|p| {
        *p.borrow_mut() = Some(parser);
    });

    Ok(true)
}

/// Close the currently opened book.
#[wasm_bindgen]
pub fn wasm_close_book() {
    CURRENT_PARSER.with(|p| {
        *p.borrow_mut() = None;
    });
}

/// Get book metadata as JSON string.
#[wasm_bindgen]
pub fn wasm_get_metadata() -> Result<String, JsError> {
    CURRENT_PARSER.with(|p| {
        let mut parser_ref = p.borrow_mut();
        let parser = parser_ref
            .as_mut()
            .ok_or_else(|| JsError::new("No book opened"))?;

        let metadata = parser
            .parse_metadata()
            .map_err(|e| JsError::new(&format!("Metadata error: {e}")))?;

        serde_json::to_string(&metadata)
            .map_err(|e| JsError::new(&format!("Serialization error: {e}")))
    })
}

/// Get table of contents as JSON string.
#[wasm_bindgen]
pub fn wasm_get_toc() -> Result<String, JsError> {
    CURRENT_PARSER.with(|p| {
        let mut parser_ref = p.borrow_mut();
        let parser = parser_ref
            .as_mut()
            .ok_or_else(|| JsError::new("No book opened"))?;

        let toc = parser
            .parse_toc()
            .map_err(|e| JsError::new(&format!("TOC error: {e}")))?;

        serde_json::to_string(&toc).map_err(|e| JsError::new(&format!("Serialization error: {e}")))
    })
}

/// Get the spine (ordered list of content paths) as JSON string.
#[wasm_bindgen]
pub fn wasm_get_spine() -> Result<String, JsError> {
    CURRENT_PARSER.with(|p| {
        let mut parser_ref = p.borrow_mut();
        let parser = parser_ref
            .as_mut()
            .ok_or_else(|| JsError::new("No book opened"))?;

        let spine = parser
            .get_spine()
            .map_err(|e| JsError::new(&format!("Spine error: {e}")))?;

        serde_json::to_string(&spine)
            .map_err(|e| JsError::new(&format!("Serialization error: {e}")))
    })
}

/// Get chapter content as JSON DOM tree.
#[wasm_bindgen]
pub fn wasm_get_chapter_content(path: &str) -> Result<String, JsError> {
    CURRENT_PARSER.with(|p| {
        let mut parser_ref = p.borrow_mut();
        let parser = parser_ref
            .as_mut()
            .ok_or_else(|| JsError::new("No book opened"))?;

        let nodes = parser
            .parse_chapter(path)
            .map_err(|e| JsError::new(&format!("Chapter error: {e}")))?;

        serde_json::to_string(&nodes)
            .map_err(|e| JsError::new(&format!("Serialization error: {e}")))
    })
}

// ─── TXT Operations ──────────────────────────────────────────────────────────

/// Parse a TXT file from raw bytes.
/// Returns JSON with { "nodes": [...], "encoding": "..." }.
#[wasm_bindgen]
pub fn wasm_parse_txt(data: &[u8]) -> Result<String, JsError> {
    let (nodes, encoding_result) =
        TxtParser::parse(data).map_err(|e| JsError::new(&format!("Parse error: {e}")))?;

    let result = serde_json::json!({
        "nodes": nodes,
        "encoding": encoding_result.encoding_name,
        "had_replacements": encoding_result.had_replacements,
    });

    serde_json::to_string(&result).map_err(|e| JsError::new(&format!("Serialization error: {e}")))
}

// ─── XHTML Parsing (standalone) ──────────────────────────────────────────────

/// Parse raw XHTML content into a DOM tree JSON string.
/// Useful for processing individual chapter content without opening an EPUB.
#[wasm_bindgen]
pub fn wasm_parse_xhtml(xhtml: &str) -> Result<String, JsError> {
    let nodes =
        parse_xhtml_to_dom(xhtml).map_err(|e| JsError::new(&format!("Parse error: {e}")))?;

    serde_json::to_string(&nodes).map_err(|e| JsError::new(&format!("Serialization error: {e}")))
}

// ─── Utility Functions ───────────────────────────────────────────────────────

/// Generate a new UUID v4 string.
#[wasm_bindgen]
pub fn wasm_generate_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasm_engine_init() {
        assert!(wasm_engine_init());
    }

    #[test]
    fn test_wasm_engine_version() {
        let version = wasm_engine_version();
        assert!(!version.is_empty());
        assert_eq!(version, "0.1.0");
    }

    #[test]
    fn test_wasm_generate_uuid() {
        let uuid1 = wasm_generate_uuid();
        let uuid2 = wasm_generate_uuid();
        assert_ne!(uuid1, uuid2);
        assert_eq!(uuid1.len(), 36); // UUID v4 format
    }

    #[test]
    fn test_wasm_parse_xhtml() {
        let xhtml = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Test</title></head>
<body>
  <h1>Hello</h1>
  <p>World</p>
</body>
</html>"#;

        let result = wasm_parse_xhtml(xhtml).unwrap();
        let nodes: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert!(!nodes.is_empty());
    }

    #[test]
    fn test_wasm_parse_txt() {
        let data = b"Hello\n\nWorld";
        let result = wasm_parse_txt(data).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["encoding"], "UTF-8");
        assert!(!parsed["nodes"].as_array().unwrap().is_empty());
    }

    // These tests require wasm target because they use JsError
    // On native targets, we test the underlying parser directly
    #[test]
    fn test_parser_no_book_opened() {
        // Verify that CURRENT_PARSER starts as None
        CURRENT_PARSER.with(|p| {
            assert!(p.borrow().is_none());
        });
    }

    #[test]
    fn test_parser_invalid_epub() {
        // Test the underlying parser directly
        let result = EpubParser::new(vec![1, 2, 3]);
        assert!(result.is_err());
    }

    #[test]
    fn test_epub_parser_lifecycle() {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        // Create a minimal test EPUB
        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut zip = ZipWriter::new(cursor);
        let options = SimpleFileOptions::default();

        zip.start_file("mimetype", options).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        zip.start_file("META-INF/container.xml", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
        )
        .unwrap();

        zip.start_file("OEBPS/content.opf", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>WASM Test</dc:title>
    <dc:creator>Test</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="uid">wasm-test-123</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#,
        )
        .unwrap();

        zip.start_file("OEBPS/nav.xhtml", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<body><nav epub:type="toc"><ol><li><a href="ch1.xhtml">Ch1</a></li></ol></nav></body>
</html>"#,
        )
        .unwrap();

        zip.start_file("OEBPS/ch1.xhtml", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<body><h1>Chapter 1</h1><p>Content here.</p></body>
</html>"#,
        )
        .unwrap();

        let epub_data = zip.finish().unwrap().into_inner();

        // Test the underlying parser directly (no JsError dependency)
        let mut parser = EpubParser::new(epub_data).unwrap();

        let metadata = parser.parse_metadata().unwrap();
        assert_eq!(metadata.title, "WASM Test");

        let toc = parser.parse_toc().unwrap();
        assert!(!toc.is_empty());

        let spine = parser.get_spine().unwrap();
        assert!(!spine.is_empty());

        let nodes = parser.parse_chapter(&spine[0]).unwrap();
        assert!(!nodes.is_empty());
    }
}
