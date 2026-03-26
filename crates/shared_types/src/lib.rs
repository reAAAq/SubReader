//! Shared types used across all crates in the reader workspace.
//!
//! This crate defines the common data structures that are shared between
//! `core_parser`, `core_state`, `core_storage`, `ffi_c`, `ffi_wasm`, etc.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Book Metadata ───────────────────────────────────────────────────────────

/// Metadata extracted from a book file (EPUB, TXT, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BookMetadata {
    /// Unique identifier for the book.
    pub id: String,
    /// Book title.
    pub title: String,
    /// List of authors.
    pub authors: Vec<String>,
    /// Language code (e.g., "en", "zh").
    pub language: Option<String>,
    /// Publication date as ISO 8601 string.
    pub publish_date: Option<String>,
    /// Reference to the cover image within the book.
    pub cover_image_ref: Option<String>,
    /// Book format.
    pub format: BookFormat,
    /// SHA-256 hash of the original file.
    pub file_hash: Option<String>,
    /// File size in bytes.
    pub file_size: Option<u64>,
}

/// Supported book formats.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BookFormat {
    Epub,
    Txt,
}

impl std::fmt::Display for BookFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BookFormat::Epub => write!(f, "epub"),
            BookFormat::Txt => write!(f, "txt"),
        }
    }
}

// ─── Table of Contents ───────────────────────────────────────────────────────

/// A single entry in the table of contents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TocEntry {
    /// Chapter/section title.
    pub title: String,
    /// Reference to the content file or anchor.
    pub href: String,
    /// Nesting level (0 = top level).
    pub level: u32,
    /// Child entries for nested TOC.
    pub children: Vec<TocEntry>,
}

// ─── DOM Tree ────────────────────────────────────────────────────────────────

/// A node in the platform-independent DOM tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DomNode {
    /// The type of this node.
    pub node_type: NodeType,
    /// CFI anchor for precise positioning.
    pub cfi_anchor: Option<String>,
    /// Text content (for text nodes).
    pub text: Option<String>,
    /// Attributes (e.g., src for images, href for links).
    pub attributes: Vec<(String, String)>,
    /// Child nodes.
    pub children: Vec<DomNode>,
}

/// Types of DOM nodes supported by the parser.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeType {
    Document,
    Heading { level: u8 },
    Paragraph,
    Text,
    Image,
    Link,
    List { ordered: bool },
    ListItem,
    Emphasis,
    Strong,
    Code,
    BlockQuote,
    Table,
    TableRow,
    TableCell,
    LineBreak,
    Span,
}

// ─── CFI Anchor ──────────────────────────────────────────────────────────────

/// A Content Fragment Identifier for precise book positioning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct CfiAnchor {
    /// The CFI string (e.g., "/6/4[chap01]!/4/2/1:0").
    pub cfi: String,
}

impl CfiAnchor {
    pub fn new(cfi: impl Into<String>) -> Self {
        Self { cfi: cfi.into() }
    }
}

// ─── Reading Progress ────────────────────────────────────────────────────────

/// Reading progress for a specific book.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReadingProgress {
    /// Book identifier.
    pub book_id: String,
    /// Current CFI position.
    pub cfi_position: String,
    /// Progress percentage (0.0 to 100.0).
    pub percentage: f64,
    /// HLC timestamp of last update.
    pub hlc_timestamp: u64,
}

// ─── Bookmark ────────────────────────────────────────────────────────────────

/// A bookmark placed by the user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bookmark {
    /// Unique bookmark identifier.
    pub id: String,
    /// Book identifier.
    pub book_id: String,
    /// CFI position of the bookmark.
    pub cfi_position: String,
    /// Optional user-provided title.
    pub title: Option<String>,
    /// Creation timestamp (HLC).
    pub created_at: u64,
}

impl Bookmark {
    pub fn new(book_id: impl Into<String>, cfi_position: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            book_id: book_id.into(),
            cfi_position: cfi_position.into(),
            title: None,
            created_at: 0,
        }
    }
}

// ─── Annotation ──────────────────────────────────────────────────────────────

/// A highlight/annotation created by the user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Annotation {
    /// Unique annotation identifier.
    pub id: String,
    /// Book identifier.
    pub book_id: String,
    /// CFI start position.
    pub cfi_start: String,
    /// CFI end position.
    pub cfi_end: String,
    /// Highlight color as RGBA (e.g., "#FFFF00FF").
    pub color_rgba: String,
    /// Optional note text.
    pub note: Option<String>,
    /// Creation timestamp (HLC).
    pub created_at: u64,
}

impl Annotation {
    pub fn new(
        book_id: impl Into<String>,
        cfi_start: impl Into<String>,
        cfi_end: impl Into<String>,
        color_rgba: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            book_id: book_id.into(),
            cfi_start: cfi_start.into(),
            cfi_end: cfi_end.into(),
            color_rgba: color_rgba.into(),
            note: None,
            created_at: 0,
        }
    }
}

// ─── User Preference ─────────────────────────────────────────────────────────

/// A user preference stored as a key-value pair.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserPreference {
    /// Preference key (e.g., "font_size", "theme").
    pub key: String,
    /// Preference value as JSON string.
    pub value: String,
    /// HLC timestamp of last update.
    pub hlc_timestamp: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_book_metadata_serialization() {
        let meta = BookMetadata {
            id: "test-id".to_string(),
            title: "Test Book".to_string(),
            authors: vec!["Author One".to_string()],
            language: Some("en".to_string()),
            publish_date: None,
            cover_image_ref: None,
            format: BookFormat::Epub,
            file_hash: None,
            file_size: Some(1024),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let deserialized: BookMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, deserialized);
    }

    #[test]
    fn test_bookmark_creation() {
        let bm = Bookmark::new("book-1", "/6/4!/4/2:0");
        assert_eq!(bm.book_id, "book-1");
        assert_eq!(bm.cfi_position, "/6/4!/4/2:0");
        assert!(bm.title.is_none());
    }

    #[test]
    fn test_annotation_creation() {
        let ann = Annotation::new("book-1", "/6/4!/4/2:0", "/6/4!/4/2:10", "#FFFF00FF");
        assert_eq!(ann.book_id, "book-1");
        assert_eq!(ann.color_rgba, "#FFFF00FF");
    }

    #[test]
    fn test_dom_node_serialization() {
        let node = DomNode {
            node_type: NodeType::Paragraph,
            cfi_anchor: Some("/6/4!/4/2".to_string()),
            text: None,
            attributes: vec![],
            children: vec![DomNode {
                node_type: NodeType::Text,
                cfi_anchor: None,
                text: Some("Hello, world!".to_string()),
                attributes: vec![],
                children: vec![],
            }],
        };
        let json = serde_json::to_string(&node).unwrap();
        let deserialized: DomNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node, deserialized);
    }

    #[test]
    fn test_cfi_anchor_new() {
        let anchor = CfiAnchor::new("/6/4[chap01]!/4/2/1:0");
        assert_eq!(anchor.cfi, "/6/4[chap01]!/4/2/1:0");

        // Test with String input
        let anchor2 = CfiAnchor::new(String::from("/6/8!/4/2:5"));
        assert_eq!(anchor2.cfi, "/6/8!/4/2:5");
    }

    #[test]
    fn test_cfi_anchor_serialization() {
        let anchor = CfiAnchor::new("/6/4!/4/2:0");
        let json = serde_json::to_string(&anchor).unwrap();
        let deserialized: CfiAnchor = serde_json::from_str(&json).unwrap();
        assert_eq!(anchor, deserialized);
    }

    #[test]
    fn test_cfi_anchor_equality_and_hash() {
        use std::collections::HashSet;
        let a1 = CfiAnchor::new("/6/4!/4/2:0");
        let a2 = CfiAnchor::new("/6/4!/4/2:0");
        let a3 = CfiAnchor::new("/6/8!/4/2:0");

        assert_eq!(a1, a2);
        assert_ne!(a1, a3);

        let mut set = HashSet::new();
        set.insert(a1.clone());
        set.insert(a2.clone());
        assert_eq!(set.len(), 1);
        set.insert(a3);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_book_format_display() {
        assert_eq!(format!("{}", BookFormat::Epub), "epub");
        assert_eq!(format!("{}", BookFormat::Txt), "txt");
    }

    #[test]
    fn test_book_format_serialization() {
        let epub = BookFormat::Epub;
        let txt = BookFormat::Txt;

        let epub_json = serde_json::to_string(&epub).unwrap();
        let txt_json = serde_json::to_string(&txt).unwrap();

        let epub_de: BookFormat = serde_json::from_str(&epub_json).unwrap();
        let txt_de: BookFormat = serde_json::from_str(&txt_json).unwrap();

        assert_eq!(epub, epub_de);
        assert_eq!(txt, txt_de);
    }

    #[test]
    fn test_reading_progress_serialization() {
        let progress = ReadingProgress {
            book_id: "book-1".to_string(),
            cfi_position: "/6/4!/4/2:0".to_string(),
            percentage: 42.5,
            hlc_timestamp: 1000,
        };
        let json = serde_json::to_string(&progress).unwrap();
        let deserialized: ReadingProgress = serde_json::from_str(&json).unwrap();
        assert_eq!(progress, deserialized);
    }

    #[test]
    fn test_user_preference_serialization() {
        let pref = UserPreference {
            key: "font_size".to_string(),
            value: "16".to_string(),
            hlc_timestamp: 1000,
        };
        let json = serde_json::to_string(&pref).unwrap();
        let deserialized: UserPreference = serde_json::from_str(&json).unwrap();
        assert_eq!(pref, deserialized);
    }

    #[test]
    fn test_bookmark_new_generates_unique_ids() {
        let bm1 = Bookmark::new("book-1", "/6/4!/4/2:0");
        let bm2 = Bookmark::new("book-1", "/6/4!/4/2:0");
        assert_ne!(bm1.id, bm2.id);
    }

    #[test]
    fn test_annotation_new_generates_unique_ids() {
        let ann1 = Annotation::new("book-1", "/6/4:0", "/6/4:10", "#FF0000FF");
        let ann2 = Annotation::new("book-1", "/6/4:0", "/6/4:10", "#FF0000FF");
        assert_ne!(ann1.id, ann2.id);
    }

    #[test]
    fn test_toc_entry_serialization() {
        let toc = TocEntry {
            title: "Chapter 1".to_string(),
            href: "chapter1.xhtml".to_string(),
            level: 0,
            children: vec![TocEntry {
                title: "Section 1.1".to_string(),
                href: "chapter1.xhtml#s1".to_string(),
                level: 1,
                children: vec![],
            }],
        };
        let json = serde_json::to_string(&toc).unwrap();
        let deserialized: TocEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(toc, deserialized);
    }

    #[test]
    fn test_node_type_variants_serialization() {
        let variants = vec![
            NodeType::Document,
            NodeType::Heading { level: 1 },
            NodeType::Heading { level: 6 },
            NodeType::Paragraph,
            NodeType::Text,
            NodeType::Image,
            NodeType::Link,
            NodeType::List { ordered: true },
            NodeType::List { ordered: false },
            NodeType::ListItem,
            NodeType::Emphasis,
            NodeType::Strong,
            NodeType::Code,
            NodeType::BlockQuote,
            NodeType::Table,
            NodeType::TableRow,
            NodeType::TableCell,
            NodeType::LineBreak,
            NodeType::Span,
        ];

        for variant in variants {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: NodeType = serde_json::from_str(&json).unwrap();
            assert_eq!(variant, deserialized);
        }
    }
}
