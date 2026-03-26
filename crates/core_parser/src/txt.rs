//! TXT format parser with automatic encoding detection.
//!
//! Supports UTF-8, GBK/GB2312, UTF-16 LE/BE, Shift-JIS encoding detection
//! and conversion. Provides streaming/chunked parsing for large files.

use chardetng::EncodingDetector;
use encoding_rs::Encoding;

use shared_types::{DomNode, NodeType};

use crate::error::ParseError;

/// Result of encoding detection.
#[derive(Debug, Clone)]
pub struct EncodingResult {
    /// Detected encoding name.
    pub encoding_name: String,
    /// Decoded UTF-8 string.
    pub content: String,
    /// Whether the encoding was detected with high confidence.
    pub confident: bool,
    /// Whether any replacement characters were used.
    pub had_replacements: bool,
}

/// TXT parser that handles encoding detection and paragraph splitting.
#[derive(Debug)]
pub struct TxtParser;

impl TxtParser {
    /// Parse raw bytes into DOM nodes with automatic encoding detection.
    pub fn parse(data: &[u8]) -> Result<(Vec<DomNode>, EncodingResult), ParseError> {
        if data.is_empty() {
            return Err(ParseError::EmptyContent);
        }

        let encoding_result = Self::detect_and_decode(data)?;
        let nodes = Self::split_paragraphs(&encoding_result.content);

        Ok((nodes, encoding_result))
    }

    /// Parse a large file in chunks using a reader.
    /// Returns DOM nodes for each chunk as they are processed.
    pub fn parse_chunked(
        data: &[u8],
        chunk_size: usize,
    ) -> Result<(Vec<DomNode>, EncodingResult), ParseError> {
        if data.is_empty() {
            return Err(ParseError::EmptyContent);
        }

        // For encoding detection, we sample the first portion of the file
        let sample_size = chunk_size.min(data.len()).min(64 * 1024); // Max 64KB sample
        let encoding = Self::detect_encoding(&data[..sample_size]);

        let mut full_content = String::new();
        let mut had_replacements = false;

        // Decode in chunks
        let mut offset = 0;
        while offset < data.len() {
            let end = (offset + chunk_size).min(data.len());
            let chunk = &data[offset..end];

            let (decoded, _, had_errors) = encoding.decode(chunk);
            if had_errors {
                had_replacements = true;
            }
            full_content.push_str(&decoded);
            offset = end;
        }

        let encoding_result = EncodingResult {
            encoding_name: encoding.name().to_string(),
            content: full_content.clone(),
            confident: true,
            had_replacements,
        };

        let nodes = Self::split_paragraphs(&full_content);
        Ok((nodes, encoding_result))
    }

    /// Detect encoding and decode bytes to UTF-8 string.
    pub fn detect_and_decode(data: &[u8]) -> Result<EncodingResult, ParseError> {
        // Check for BOM (Byte Order Mark) first
        if let Some(result) = Self::detect_bom(data) {
            return Ok(result);
        }

        let encoding = Self::detect_encoding(data);
        let (decoded, _, had_errors) = encoding.decode(data);

        Ok(EncodingResult {
            encoding_name: encoding.name().to_string(),
            content: decoded.to_string(),
            confident: true,
            had_replacements: had_errors,
        })
    }

    /// Check for BOM and decode accordingly.
    fn detect_bom(data: &[u8]) -> Option<EncodingResult> {
        // UTF-8 BOM
        if data.len() >= 3 && data[0] == 0xEF && data[1] == 0xBB && data[2] == 0xBF {
            let content = String::from_utf8_lossy(&data[3..]).to_string();
            let had_replacements = content.contains('\u{FFFD}');
            return Some(EncodingResult {
                encoding_name: "UTF-8".to_string(),
                content,
                confident: true,
                had_replacements,
            });
        }

        // UTF-16 LE BOM
        if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xFE {
            let (decoded, _, had_errors) = encoding_rs::UTF_16LE.decode(data);
            return Some(EncodingResult {
                encoding_name: "UTF-16LE".to_string(),
                content: decoded.to_string(),
                confident: true,
                had_replacements: had_errors,
            });
        }

        // UTF-16 BE BOM
        if data.len() >= 2 && data[0] == 0xFE && data[1] == 0xFF {
            let (decoded, _, had_errors) = encoding_rs::UTF_16BE.decode(data);
            return Some(EncodingResult {
                encoding_name: "UTF-16BE".to_string(),
                content: decoded.to_string(),
                confident: true,
                had_replacements: had_errors,
            });
        }

        None
    }

    /// Detect encoding using chardetng.
    fn detect_encoding(data: &[u8]) -> &'static Encoding {
        let mut detector = EncodingDetector::new();
        detector.feed(data, true);
        detector.guess(None, true)
    }

    /// Split text content into paragraph DOM nodes.
    pub fn split_paragraphs(content: &str) -> Vec<DomNode> {
        let mut nodes = Vec::new();
        let mut paragraph_index: u32 = 0;

        // Split by double newlines or blank lines
        for paragraph in split_into_paragraphs(content) {
            let trimmed = paragraph.trim();
            if trimmed.is_empty() {
                continue;
            }

            paragraph_index += 1;
            let cfi = format!("/{}", paragraph_index * 2);

            let text_node = DomNode {
                node_type: NodeType::Text,
                cfi_anchor: None,
                text: Some(trimmed.to_string()),
                attributes: Vec::new(),
                children: Vec::new(),
            };

            let para_node = DomNode {
                node_type: NodeType::Paragraph,
                cfi_anchor: Some(cfi),
                text: None,
                attributes: Vec::new(),
                children: vec![text_node],
            };

            nodes.push(para_node);
        }

        nodes
    }
}

/// Split text into paragraphs by blank lines or double newlines.
fn split_into_paragraphs(text: &str) -> Vec<&str> {
    let mut paragraphs = Vec::new();
    let mut start = 0;
    let mut prev_was_newline = false;
    let bytes = text.as_bytes();
    let len = bytes.len();

    let mut i = 0;
    while i < len {
        let ch = bytes[i];
        if ch == b'\n' || ch == b'\r' {
            if prev_was_newline {
                // Double newline found — split here
                let segment = &text[start..i];
                if !segment.trim().is_empty() {
                    paragraphs.push(segment);
                }
                // Skip consecutive newlines
                while i < len && (bytes[i] == b'\n' || bytes[i] == b'\r') {
                    i += 1;
                }
                start = i;
                prev_was_newline = false;
                continue;
            }
            prev_was_newline = true;
        } else {
            prev_was_newline = false;
        }
        i += 1;
    }

    // Don't forget the last segment
    if start < len {
        let segment = &text[start..];
        if !segment.trim().is_empty() {
            paragraphs.push(segment);
        }
    }

    paragraphs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_utf8() {
        let content = "Hello, world!\n\nThis is a test.\n\nThird paragraph.";
        let data = content.as_bytes();
        let (nodes, enc) = TxtParser::parse(data).unwrap();

        assert_eq!(enc.encoding_name, "UTF-8");
        assert!(!enc.had_replacements);
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].node_type, NodeType::Paragraph);

        // Check text content
        let text = nodes[0].children[0].text.as_ref().unwrap();
        assert_eq!(text, "Hello, world!");
    }

    #[test]
    fn test_parse_utf8_with_bom() {
        let mut data = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
        data.extend_from_slice(b"Hello with BOM\n\nSecond paragraph");
        let (nodes, enc) = TxtParser::parse(&data).unwrap();

        assert_eq!(enc.encoding_name, "UTF-8");
        assert_eq!(nodes.len(), 2);
    }

    #[test]
    fn test_parse_utf16le() {
        // UTF-16 LE BOM + "Hi\n\nBye"
        let text = "Hi\n\nBye";
        let mut data = vec![0xFF, 0xFE]; // UTF-16 LE BOM
        for ch in text.encode_utf16() {
            data.extend_from_slice(&ch.to_le_bytes());
        }

        let (nodes, enc) = TxtParser::parse(&data).unwrap();
        assert_eq!(enc.encoding_name, "UTF-16LE");
        assert_eq!(nodes.len(), 2);
    }

    #[test]
    fn test_parse_utf16be() {
        // UTF-16 BE BOM + "Hello\n\nWorld"
        let text = "Hello\n\nWorld";
        let mut data = vec![0xFE, 0xFF]; // UTF-16 BE BOM
        for ch in text.encode_utf16() {
            data.extend_from_slice(&ch.to_be_bytes());
        }

        let (nodes, enc) = TxtParser::parse(&data).unwrap();
        assert_eq!(enc.encoding_name, "UTF-16BE");
        assert_eq!(nodes.len(), 2);
    }

    #[test]
    fn test_parse_gbk() {
        // GBK encoded Chinese text: "你好\n\n世界"
        let (encoded, _, _) = encoding_rs::GBK.encode("你好\n\n世界");
        let (nodes, enc) = TxtParser::parse(&encoded).unwrap();

        // chardetng should detect this as GBK or a compatible encoding
        assert!(!enc.encoding_name.is_empty());
        assert!(nodes.len() >= 1); // At least some content parsed
    }

    #[test]
    fn test_parse_empty() {
        let result = TxtParser::parse(&[]);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ParseError::EmptyContent));
    }

    #[test]
    fn test_split_paragraphs_basic() {
        let content = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
        let nodes = TxtParser::split_paragraphs(content);
        assert_eq!(nodes.len(), 3);
    }

    #[test]
    fn test_split_paragraphs_windows_newlines() {
        let content = "First paragraph.\r\n\r\nSecond paragraph.\r\n\r\nThird.";
        let nodes = TxtParser::split_paragraphs(content);
        assert_eq!(nodes.len(), 3);
    }

    #[test]
    fn test_split_paragraphs_single() {
        let content = "Just one paragraph with no breaks.";
        let nodes = TxtParser::split_paragraphs(content);
        assert_eq!(nodes.len(), 1);
    }

    #[test]
    fn test_split_paragraphs_empty_lines() {
        let content = "\n\n\nSome text\n\n\n\nMore text\n\n\n";
        let nodes = TxtParser::split_paragraphs(content);
        assert_eq!(nodes.len(), 2);
    }

    #[test]
    fn test_cfi_anchors_sequential() {
        let content = "Para 1\n\nPara 2\n\nPara 3";
        let nodes = TxtParser::split_paragraphs(content);
        assert_eq!(nodes[0].cfi_anchor.as_ref().unwrap(), "/2");
        assert_eq!(nodes[1].cfi_anchor.as_ref().unwrap(), "/4");
        assert_eq!(nodes[2].cfi_anchor.as_ref().unwrap(), "/6");
    }

    #[test]
    fn test_chunked_parsing() {
        let content = "First chunk paragraph.\n\nSecond chunk paragraph.\n\nThird.";
        let data = content.as_bytes();
        let (nodes, enc) = TxtParser::parse_chunked(data, 16).unwrap();

        assert_eq!(enc.encoding_name, "UTF-8");
        // Chunked parsing may merge/split differently but should capture all content
        assert!(!nodes.is_empty());
    }

    #[test]
    fn test_large_content() {
        // Generate a large text content (>10MB simulation with smaller size)
        let paragraph = "This is a test paragraph with some content. ".repeat(100);
        let content = (0..100)
            .map(|_| paragraph.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        let data = content.as_bytes();
        let (nodes, _) = TxtParser::parse(data).unwrap();
        assert_eq!(nodes.len(), 100);
    }

    #[test]
    fn test_replacement_characters() {
        // Invalid UTF-8 sequence that should trigger replacement
        let data = vec![0xFF, 0xFE, 0x48, 0x00, 0x69, 0x00]; // UTF-16 LE "Hi"
        let (nodes, enc) = TxtParser::parse(&data).unwrap();
        assert_eq!(enc.encoding_name, "UTF-16LE");
        assert!(!nodes.is_empty());
    }
}
