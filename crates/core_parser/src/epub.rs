//! EPUB format parser.
//!
//! Parses EPUB 2/3 files into structured metadata, table of contents,
//! and a platform-independent DOM tree. No WebView or browser engine required.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use quick_xml::events::Event;
use quick_xml::Reader;
use zip::ZipArchive;

use shared_types::{BookFormat, BookMetadata, DomNode, NodeType, TocEntry};

use crate::error::ParseError;

/// EPUB parser that extracts metadata, TOC, and content from EPUB files.
pub struct EpubParser {
    /// Raw bytes of the EPUB file.
    data: Vec<u8>,
    /// Parsed ZIP archive.
    archive: ZipArchive<Cursor<Vec<u8>>>,
    /// Path to the OPF file within the archive.
    opf_path: String,
    /// Base directory of the OPF file (for resolving relative paths).
    opf_base: String,
}

impl std::fmt::Debug for EpubParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpubParser")
            .field("opf_path", &self.opf_path)
            .field("opf_base", &self.opf_base)
            .finish()
    }
}

impl EpubParser {
    /// Create a new EPUB parser from raw file bytes.
    pub fn new(data: Vec<u8>) -> Result<Self, ParseError> {
        if data.is_empty() {
            return Err(ParseError::EmptyContent);
        }

        let cursor = Cursor::new(data.clone());
        let archive = ZipArchive::new(cursor)?;

        let mut parser = Self {
            data,
            archive,
            opf_path: String::new(),
            opf_base: String::new(),
        };

        parser.locate_opf()?;
        Ok(parser)
    }

    /// Locate the OPF file by parsing META-INF/container.xml.
    fn locate_opf(&mut self) -> Result<(), ParseError> {
        let container_xml = self.read_file_from_archive("META-INF/container.xml")?;
        let container_str = String::from_utf8(container_xml)
            .map_err(|e| ParseError::InvalidEpub(format!("Invalid UTF-8 in container.xml: {e}")))?;

        let mut reader = Reader::from_str(&container_str);
        reader.config_mut().trim_text(true);

        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e))
                    if e.name().as_ref() == b"rootfile" =>
                {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"full-path" {
                            let path = String::from_utf8(attr.value.to_vec()).map_err(|e| {
                                ParseError::InvalidEpub(format!("Invalid OPF path: {e}"))
                            })?;
                            self.opf_base = path
                                .rfind('/')
                                .map(|i| path[..=i].to_string())
                                .unwrap_or_default();
                            self.opf_path = path;
                            return Ok(());
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(ParseError::InvalidXml(format!("container.xml: {e}"))),
                _ => {}
            }
            buf.clear();
        }

        Err(ParseError::InvalidEpub(
            "No rootfile found in container.xml".to_string(),
        ))
    }

    /// Extract book metadata from the OPF file.
    pub fn parse_metadata(&mut self) -> Result<BookMetadata, ParseError> {
        let opf_data = self.read_file_from_archive(&self.opf_path.clone())?;
        let opf_str = String::from_utf8(opf_data)
            .map_err(|e| ParseError::InvalidEpub(format!("Invalid UTF-8 in OPF: {e}")))?;

        let mut reader = Reader::from_str(&opf_str);
        reader.config_mut().trim_text(true);

        let mut title = String::new();
        let mut authors = Vec::new();
        let mut language = None;
        let mut publish_date = None;
        let mut cover_id = None;
        let mut id = String::new();

        let mut buf = Vec::new();
        let mut current_tag = String::new();
        let mut in_metadata = false;

        // First pass: find cover meta
        let mut meta_reader = Reader::from_str(&opf_str);
        meta_reader.config_mut().trim_text(true);
        let mut meta_buf = Vec::new();
        loop {
            match meta_reader.read_event_into(&mut meta_buf) {
                Ok(Event::Empty(ref e)) if e.name().as_ref() == b"meta" => {
                    let mut is_cover = false;
                    let mut content_val = None;
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"name" if attr.value.as_ref() == b"cover" => is_cover = true,
                            b"content" => {
                                content_val = Some(String::from_utf8_lossy(&attr.value).to_string())
                            }
                            _ => {}
                        }
                    }
                    if is_cover {
                        cover_id = content_val;
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
            meta_buf.clear();
        }

        // Second pass: extract metadata elements
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let local_name = e.name().as_ref().to_vec();
                    let tag = String::from_utf8_lossy(&local_name).to_string();
                    // Strip namespace prefix
                    let tag = tag.rsplit(':').next().unwrap_or(&tag).to_string();

                    if tag == "metadata" {
                        in_metadata = true;
                    }
                    if in_metadata {
                        current_tag = tag;
                    }
                }
                Ok(Event::Text(ref e)) if in_metadata => {
                    let text = e.unescape().unwrap_or_default().to_string();
                    match current_tag.as_str() {
                        "title" => title = text,
                        "creator" => authors.push(text),
                        "language" => language = Some(text),
                        "date" => publish_date = Some(text),
                        "identifier" => {
                            if id.is_empty() {
                                id = text;
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(ref e)) => {
                    let local_name = e.name().as_ref().to_vec();
                    let tag = String::from_utf8_lossy(&local_name).to_string();
                    let tag = tag.rsplit(':').next().unwrap_or(&tag);
                    if tag == "metadata" {
                        in_metadata = false;
                    }
                    current_tag.clear();
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(ParseError::InvalidXml(format!("OPF metadata: {e}"))),
                _ => {}
            }
            buf.clear();
        }

        // Compute file hash
        use sha2::{Digest, Sha256};
        let hash = format!("{:x}", Sha256::digest(&self.data));

        if id.is_empty() {
            id = hash.clone();
        }

        Ok(BookMetadata {
            id,
            title,
            authors,
            language,
            publish_date,
            cover_image_ref: cover_id,
            format: BookFormat::Epub,
            file_hash: Some(hash),
            file_size: Some(self.data.len() as u64),
        })
    }

    /// Parse the table of contents from NCX or NAV document.
    pub fn parse_toc(&mut self) -> Result<Vec<TocEntry>, ParseError> {
        let opf_data = self.read_file_from_archive(&self.opf_path.clone())?;
        let opf_str = String::from_utf8(opf_data)
            .map_err(|e| ParseError::InvalidEpub(format!("Invalid UTF-8 in OPF: {e}")))?;

        // Find the NCX or NAV file reference in the OPF manifest
        let toc_path = self.find_toc_path(&opf_str)?;
        let toc_data = self.read_file_from_archive(&toc_path)?;
        let toc_str = String::from_utf8(toc_data)
            .map_err(|e| ParseError::InvalidEpub(format!("Invalid UTF-8 in TOC: {e}")))?;

        if toc_path.ends_with(".ncx") {
            self.parse_ncx_toc(&toc_str)
        } else {
            self.parse_nav_toc(&toc_str)
        }
    }

    /// Find the TOC file path from the OPF manifest.
    fn find_toc_path(&self, opf_str: &str) -> Result<String, ParseError> {
        let mut reader = Reader::from_str(opf_str);
        reader.config_mut().trim_text(true);

        let mut manifest_items: HashMap<String, (String, String)> = HashMap::new();
        let mut toc_id = None;
        let mut in_manifest = false;

        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if name == "manifest" || name.ends_with(":manifest") {
                        in_manifest = true;
                    }
                    if name == "spine" || name.ends_with(":spine") {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"toc" {
                                toc_id = Some(String::from_utf8_lossy(&attr.value).to_string());
                            }
                        }
                    }
                }
                Ok(Event::Empty(ref e)) if in_manifest => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if name == "item" || name.ends_with(":item") {
                        let mut item_id = String::new();
                        let mut href = String::new();
                        let mut media_type = String::new();
                        let mut properties = String::new();
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"id" => item_id = String::from_utf8_lossy(&attr.value).to_string(),
                                b"href" => href = String::from_utf8_lossy(&attr.value).to_string(),
                                b"media-type" => {
                                    media_type = String::from_utf8_lossy(&attr.value).to_string()
                                }
                                b"properties" => {
                                    properties = String::from_utf8_lossy(&attr.value).to_string()
                                }
                                _ => {}
                            }
                        }
                        manifest_items
                            .insert(item_id, (href, format!("{media_type}|{properties}")));
                    }
                }
                Ok(Event::End(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if name == "manifest" || name.ends_with(":manifest") {
                        in_manifest = false;
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(ParseError::InvalidXml(format!("OPF manifest: {e}"))),
                _ => {}
            }
            buf.clear();
        }

        // Try to find NAV document (EPUB 3)
        for (href, props) in manifest_items.values() {
            if props.contains("nav") {
                return Ok(format!("{}{}", self.opf_base, href));
            }
        }

        // Fall back to NCX (EPUB 2)
        if let Some(ncx_id) = toc_id {
            if let Some((href, _)) = manifest_items.get(&ncx_id) {
                return Ok(format!("{}{}", self.opf_base, href));
            }
        }

        // Try to find any .ncx file
        for (href, media) in manifest_items.values() {
            if media.contains("application/x-dtbncx+xml") {
                return Ok(format!("{}{}", self.opf_base, href));
            }
        }

        Err(ParseError::InvalidEpub(
            "No TOC (NCX or NAV) found in manifest".to_string(),
        ))
    }

    /// Parse NCX format table of contents.
    fn parse_ncx_toc(&self, ncx_str: &str) -> Result<Vec<TocEntry>, ParseError> {
        let mut reader = Reader::from_str(ncx_str);
        reader.config_mut().trim_text(true);

        let mut entries = Vec::new();
        let mut buf = Vec::new();
        let mut stack: Vec<Vec<TocEntry>> = vec![Vec::new()];
        let mut current_title = String::new();
        let mut current_href = String::new();
        let mut in_text = false;
        let mut depth: u32 = 0;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if name == "navPoint" || name.ends_with(":navPoint") {
                        depth += 1;
                        stack.push(Vec::new());
                        current_title.clear();
                        current_href.clear();
                    } else if name == "text" || name.ends_with(":text") {
                        in_text = true;
                    }
                }
                Ok(Event::Empty(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if name == "content" || name.ends_with(":content") {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"src" {
                                current_href = String::from_utf8_lossy(&attr.value).to_string();
                            }
                        }
                    }
                }
                Ok(Event::Text(ref e)) if in_text => {
                    current_title = e.unescape().unwrap_or_default().to_string();
                }
                Ok(Event::End(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if name == "text" || name.ends_with(":text") {
                        in_text = false;
                    } else if name == "navPoint" || name.ends_with(":navPoint") {
                        let children = stack.pop().unwrap_or_default();
                        depth = depth.saturating_sub(1);
                        let entry = TocEntry {
                            title: current_title.clone(),
                            href: current_href.clone(),
                            level: depth,
                            children,
                        };
                        if let Some(parent) = stack.last_mut() {
                            parent.push(entry);
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(ParseError::InvalidXml(format!("NCX: {e}"))),
                _ => {}
            }
            buf.clear();
        }

        if let Some(top) = stack.pop() {
            entries = top;
        }

        Ok(entries)
    }

    /// Parse NAV format table of contents (EPUB 3).
    fn parse_nav_toc(&self, nav_str: &str) -> Result<Vec<TocEntry>, ParseError> {
        // Simplified NAV parser: extract <a> elements within <nav epub:type="toc">
        let mut reader = Reader::from_str(nav_str);
        reader.config_mut().trim_text(true);

        let mut entries = Vec::new();
        let mut buf = Vec::new();
        let mut in_nav_toc = false;
        let mut current_href = String::new();
        let mut in_a = false;
        let mut current_title = String::new();
        let mut depth: u32 = 0;
        let mut ol_depth: u32 = 0;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if name == "nav" {
                        for attr in e.attributes().flatten() {
                            let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                            if key.contains("type") {
                                let val = String::from_utf8_lossy(&attr.value).to_string();
                                if val == "toc" {
                                    in_nav_toc = true;
                                }
                            }
                        }
                    } else if in_nav_toc && name == "ol" {
                        ol_depth += 1;
                        depth = ol_depth.saturating_sub(1);
                    } else if in_nav_toc && name == "a" {
                        in_a = true;
                        current_title.clear();
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"href" {
                                current_href = String::from_utf8_lossy(&attr.value).to_string();
                            }
                        }
                    }
                }
                Ok(Event::Text(ref e)) if in_a => {
                    current_title.push_str(&e.unescape().unwrap_or_default());
                }
                Ok(Event::End(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if name == "nav" && in_nav_toc {
                        in_nav_toc = false;
                    } else if name == "ol" && in_nav_toc {
                        ol_depth = ol_depth.saturating_sub(1);
                    } else if name == "a" && in_a {
                        in_a = false;
                        entries.push(TocEntry {
                            title: current_title.clone(),
                            href: current_href.clone(),
                            level: depth,
                            children: Vec::new(),
                        });
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(ParseError::InvalidXml(format!("NAV: {e}"))),
                _ => {}
            }
            buf.clear();
        }

        Ok(entries)
    }

    /// Get the spine (ordered list of content document paths).
    pub fn get_spine(&mut self) -> Result<Vec<String>, ParseError> {
        let opf_data = self.read_file_from_archive(&self.opf_path.clone())?;
        let opf_str = String::from_utf8(opf_data)
            .map_err(|e| ParseError::InvalidEpub(format!("Invalid UTF-8 in OPF: {e}")))?;

        let mut reader = Reader::from_str(&opf_str);
        reader.config_mut().trim_text(true);

        let mut manifest: HashMap<String, String> = HashMap::new();
        let mut spine_idrefs: Vec<String> = Vec::new();
        let mut in_manifest = false;
        let mut in_spine = false;

        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if name == "manifest" || name.ends_with(":manifest") {
                        in_manifest = true;
                    } else if name == "spine" || name.ends_with(":spine") {
                        in_spine = true;
                    }
                }
                Ok(Event::Empty(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if in_manifest && (name == "item" || name.ends_with(":item")) {
                        let mut item_id = String::new();
                        let mut href = String::new();
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"id" => item_id = String::from_utf8_lossy(&attr.value).to_string(),
                                b"href" => href = String::from_utf8_lossy(&attr.value).to_string(),
                                _ => {}
                            }
                        }
                        manifest.insert(item_id, href);
                    } else if in_spine && (name == "itemref" || name.ends_with(":itemref")) {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"idref" {
                                spine_idrefs.push(String::from_utf8_lossy(&attr.value).to_string());
                            }
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if name == "manifest" || name.ends_with(":manifest") {
                        in_manifest = false;
                    } else if name == "spine" || name.ends_with(":spine") {
                        in_spine = false;
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(ParseError::InvalidXml(format!("OPF spine: {e}"))),
                _ => {}
            }
            buf.clear();
        }

        let paths: Vec<String> = spine_idrefs
            .iter()
            .filter_map(|idref| {
                manifest
                    .get(idref)
                    .map(|href| format!("{}{}", self.opf_base, href))
            })
            .collect();

        Ok(paths)
    }

    /// Get the cover image data by resolving the cover item id from the manifest.
    ///
    /// Returns the raw image bytes (JPEG, PNG, etc.) or an error if no cover is found.
    pub fn get_cover_image(&mut self, cover_id: &str) -> Result<Vec<u8>, ParseError> {
        let opf_data = self.read_file_from_archive(&self.opf_path.clone())?;
        let opf_str = String::from_utf8(opf_data)
            .map_err(|e| ParseError::InvalidEpub(format!("Invalid UTF-8 in OPF: {e}")))?;

        let mut reader = Reader::from_str(&opf_str);
        reader.config_mut().trim_text(true);

        let mut cover_href: Option<String> = None;
        let mut in_manifest = false;
        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if name == "manifest" || name.ends_with(":manifest") {
                        in_manifest = true;
                    }
                }
                Ok(Event::Empty(ref e)) if in_manifest => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if name == "item" || name.ends_with(":item") {
                        let mut item_id = String::new();
                        let mut href = String::new();
                        let mut properties = String::new();
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"id" => item_id = String::from_utf8_lossy(&attr.value).to_string(),
                                b"href" => href = String::from_utf8_lossy(&attr.value).to_string(),
                                b"properties" => {
                                    properties = String::from_utf8_lossy(&attr.value).to_string()
                                }
                                _ => {}
                            }
                        }
                        // Match by item id or by cover-image property (EPUB 3)
                        if item_id == cover_id || properties.contains("cover-image") {
                            cover_href = Some(href);
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if name == "manifest" || name.ends_with(":manifest") {
                        in_manifest = false;
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(ParseError::InvalidXml(format!("OPF manifest: {e}"))),
                _ => {}
            }
            buf.clear();
        }

        let href = cover_href.ok_or_else(|| {
            ParseError::InvalidEpub(format!("Cover item '{}' not found in manifest", cover_id))
        })?;

        let full_path = format!("{}{}", self.opf_base, href);
        self.read_file_from_archive(&full_path)
    }

    /// Parse a single XHTML content document into a DOM tree.
    pub fn parse_chapter(&mut self, path: &str) -> Result<Vec<DomNode>, ParseError> {
        let data = self.read_file_from_archive(path)?;
        let content = String::from_utf8(data)
            .map_err(|e| ParseError::InvalidEpub(format!("Invalid UTF-8 in chapter: {e}")))?;

        parse_xhtml_to_dom(&content)
    }

    /// Read a file from the ZIP archive.
    fn read_file_from_archive(&mut self, path: &str) -> Result<Vec<u8>, ParseError> {
        let mut file = self.archive.by_name(path).map_err(|e| {
            ParseError::InvalidEpub(format!("File not found in archive: {path}: {e}"))
        })?;

        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| ParseError::IoError(format!("Failed to read {path}: {e}")))?;

        Ok(buf)
    }
}

/// Parse XHTML content into a list of DOM nodes.
pub fn parse_xhtml_to_dom(xhtml: &str) -> Result<Vec<DomNode>, ParseError> {
    let mut reader = Reader::from_str(xhtml);
    reader.config_mut().trim_text(false);

    let mut root_children: Vec<DomNode> = Vec::new();
    let mut stack: Vec<DomNode> = Vec::new();
    let mut cfi_counters: Vec<u32> = vec![0];
    let mut in_body = false;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let tag = tag.rsplit(':').next().unwrap_or(&tag).to_string();

                if tag == "body" {
                    in_body = true;
                    buf.clear();
                    continue;
                }

                if !in_body {
                    buf.clear();
                    continue;
                }

                // Increment CFI counter at current level
                if let Some(counter) = cfi_counters.last_mut() {
                    *counter += 1;
                }

                let node_type = tag_to_node_type(&tag);
                let mut attributes = Vec::new();
                for attr in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                    let val = String::from_utf8_lossy(&attr.value).to_string();
                    attributes.push((key, val));
                }

                let cfi = build_cfi(&cfi_counters);
                let node = DomNode {
                    node_type,
                    cfi_anchor: Some(cfi),
                    text: None,
                    attributes,
                    children: Vec::new(),
                };

                stack.push(node);
                cfi_counters.push(0);
            }
            Ok(Event::Text(ref e)) if in_body => {
                let text = e.unescape().unwrap_or_default().to_string();
                if !text.trim().is_empty() {
                    if let Some(counter) = cfi_counters.last_mut() {
                        *counter += 1;
                    }
                    let cfi = build_cfi(&cfi_counters);
                    let text_node = DomNode {
                        node_type: NodeType::Text,
                        cfi_anchor: Some(cfi),
                        text: Some(text),
                        attributes: Vec::new(),
                        children: Vec::new(),
                    };
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(text_node);
                    } else {
                        root_children.push(text_node);
                    }
                }
            }
            Ok(Event::Empty(ref e)) if in_body => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let tag = tag.rsplit(':').next().unwrap_or(&tag).to_string();

                if let Some(counter) = cfi_counters.last_mut() {
                    *counter += 1;
                }

                let node_type = tag_to_node_type(&tag);
                let mut attributes = Vec::new();
                for attr in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                    let val = String::from_utf8_lossy(&attr.value).to_string();
                    attributes.push((key, val));
                }

                let cfi = build_cfi(&cfi_counters);
                let node = DomNode {
                    node_type,
                    cfi_anchor: Some(cfi),
                    text: None,
                    attributes,
                    children: Vec::new(),
                };

                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    root_children.push(node);
                }
            }
            Ok(Event::End(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let tag = tag.rsplit(':').next().unwrap_or(&tag).to_string();

                if tag == "body" {
                    in_body = false;
                    buf.clear();
                    continue;
                }

                if !in_body {
                    buf.clear();
                    continue;
                }

                cfi_counters.pop();

                if let Some(node) = stack.pop() {
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(node);
                    } else {
                        root_children.push(node);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(ParseError::InvalidXml(format!("XHTML content: {e}"))),
            _ => {}
        }
        buf.clear();
    }

    Ok(root_children)
}

/// Map HTML tag names to NodeType.
fn tag_to_node_type(tag: &str) -> NodeType {
    match tag.to_lowercase().as_str() {
        "h1" => NodeType::Heading { level: 1 },
        "h2" => NodeType::Heading { level: 2 },
        "h3" => NodeType::Heading { level: 3 },
        "h4" => NodeType::Heading { level: 4 },
        "h5" => NodeType::Heading { level: 5 },
        "h6" => NodeType::Heading { level: 6 },
        "p" => NodeType::Paragraph,
        "img" | "image" => NodeType::Image,
        "a" => NodeType::Link,
        "ul" => NodeType::List { ordered: false },
        "ol" => NodeType::List { ordered: true },
        "li" => NodeType::ListItem,
        "em" | "i" => NodeType::Emphasis,
        "strong" | "b" => NodeType::Strong,
        "code" | "pre" => NodeType::Code,
        "blockquote" => NodeType::BlockQuote,
        "table" => NodeType::Table,
        "tr" => NodeType::TableRow,
        "td" | "th" => NodeType::TableCell,
        "br" => NodeType::LineBreak,
        "span" | "div" | "section" | "article" | "aside" | "header" | "footer" | "main"
        | "figure" | "figcaption" | "nav" | "details" | "summary" => NodeType::Span,
        _ => NodeType::Span,
    }
}

/// Build a CFI string from the counter stack.
fn build_cfi(counters: &[u32]) -> String {
    let parts: Vec<String> = counters
        .iter()
        .skip(1) // skip root level
        .map(|c| format!("/{}", c * 2)) // EPUB CFI uses even numbers
        .collect();
    if parts.is_empty() {
        "/2".to_string()
    } else {
        parts.join("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tag_to_node_type() {
        assert_eq!(tag_to_node_type("p"), NodeType::Paragraph);
        assert_eq!(tag_to_node_type("h1"), NodeType::Heading { level: 1 });
        assert_eq!(tag_to_node_type("h3"), NodeType::Heading { level: 3 });
        assert_eq!(tag_to_node_type("img"), NodeType::Image);
        assert_eq!(tag_to_node_type("a"), NodeType::Link);
        assert_eq!(tag_to_node_type("ul"), NodeType::List { ordered: false });
        assert_eq!(tag_to_node_type("ol"), NodeType::List { ordered: true });
        assert_eq!(tag_to_node_type("em"), NodeType::Emphasis);
        assert_eq!(tag_to_node_type("strong"), NodeType::Strong);
        assert_eq!(tag_to_node_type("code"), NodeType::Code);
        assert_eq!(tag_to_node_type("blockquote"), NodeType::BlockQuote);
        assert_eq!(tag_to_node_type("table"), NodeType::Table);
        assert_eq!(tag_to_node_type("div"), NodeType::Span);
    }

    #[test]
    fn test_build_cfi() {
        assert_eq!(build_cfi(&[0]), "/2");
        assert_eq!(build_cfi(&[0, 1]), "/2");
        assert_eq!(build_cfi(&[0, 2]), "/4");
        assert_eq!(build_cfi(&[0, 1, 3]), "/2/6");
    }

    #[test]
    fn test_parse_xhtml_simple() {
        let xhtml = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Test</title></head>
<body>
  <h1>Chapter 1</h1>
  <p>Hello, <em>world</em>!</p>
</body>
</html>"#;

        let nodes = parse_xhtml_to_dom(xhtml).unwrap();
        assert!(!nodes.is_empty());

        // First node should be h1
        assert_eq!(nodes[0].node_type, NodeType::Heading { level: 1 });
        // Second node should be p
        assert_eq!(nodes[1].node_type, NodeType::Paragraph);
    }

    #[test]
    fn test_parse_xhtml_with_attributes() {
        let xhtml = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Test</title></head>
<body>
  <p>Click <a href="http://example.com">here</a></p>
  <img src="image.png" alt="test"/>
</body>
</html>"#;

        let nodes = parse_xhtml_to_dom(xhtml).unwrap();
        assert!(!nodes.is_empty());
    }

    #[test]
    fn test_parse_xhtml_empty_body() {
        let xhtml = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Test</title></head>
<body>
</body>
</html>"#;

        let nodes = parse_xhtml_to_dom(xhtml).unwrap();
        assert!(nodes.is_empty());
    }

    #[test]
    fn test_epub_parser_empty_data() {
        let result = EpubParser::new(vec![]);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ParseError::EmptyContent));
    }

    #[test]
    fn test_epub_parser_invalid_zip() {
        let result = EpubParser::new(vec![1, 2, 3, 4]);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ParseError::InvalidZip(_)));
    }

    /// Helper to create a minimal valid EPUB in memory for testing.
    fn create_test_epub() -> Vec<u8> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut zip = ZipWriter::new(cursor);
        let options = SimpleFileOptions::default();

        // mimetype (must be first, uncompressed)
        zip.start_file("mimetype", options).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        // container.xml
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

        // content.opf
        zip.start_file("OEBPS/content.opf", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Test Book</dc:title>
    <dc:creator>Test Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="uid">test-isbn-123</dc:identifier>
    <dc:date>2024-01-01</dc:date>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="chap01" href="chapter01.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chap01"/>
  </spine>
</package>"#,
        )
        .unwrap();

        // nav.xhtml
        zip.start_file("OEBPS/nav.xhtml", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Navigation</title></head>
<body>
  <nav epub:type="toc">
    <ol>
      <li><a href="chapter01.xhtml">Chapter 1: Introduction</a></li>
    </ol>
  </nav>
</body>
</html>"#,
        )
        .unwrap();

        // chapter01.xhtml
        zip.start_file("OEBPS/chapter01.xhtml", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter 1</title></head>
<body>
  <h1>Chapter 1: Introduction</h1>
  <p>This is the first paragraph of the test book.</p>
  <p>This is the <em>second</em> paragraph with <strong>formatting</strong>.</p>
</body>
</html>"#,
        )
        .unwrap();

        let cursor = zip.finish().unwrap();
        cursor.into_inner()
    }

    #[test]
    fn test_epub_parse_metadata() {
        let epub_data = create_test_epub();
        let mut parser = EpubParser::new(epub_data).unwrap();
        let metadata = parser.parse_metadata().unwrap();

        assert_eq!(metadata.title, "Test Book");
        assert_eq!(metadata.authors, vec!["Test Author"]);
        assert_eq!(metadata.language, Some("en".to_string()));
        assert_eq!(metadata.publish_date, Some("2024-01-01".to_string()));
        assert_eq!(metadata.format, BookFormat::Epub);
        assert!(metadata.file_hash.is_some());
        assert!(metadata.file_size.is_some());
    }

    #[test]
    fn test_epub_parse_toc() {
        let epub_data = create_test_epub();
        let mut parser = EpubParser::new(epub_data).unwrap();
        let toc = parser.parse_toc().unwrap();

        assert!(!toc.is_empty());
        assert_eq!(toc[0].title, "Chapter 1: Introduction");
        assert_eq!(toc[0].href, "chapter01.xhtml");
    }

    #[test]
    fn test_epub_get_spine() {
        let epub_data = create_test_epub();
        let mut parser = EpubParser::new(epub_data).unwrap();
        let spine = parser.get_spine().unwrap();

        assert_eq!(spine.len(), 1);
        assert!(spine[0].contains("chapter01.xhtml"));
    }

    #[test]
    fn test_epub_parse_chapter() {
        let epub_data = create_test_epub();
        let mut parser = EpubParser::new(epub_data).unwrap();
        let spine = parser.get_spine().unwrap();
        let nodes = parser.parse_chapter(&spine[0]).unwrap();

        assert!(!nodes.is_empty());
        // Should have h1 and two p elements
        assert!(nodes.len() >= 3);
        assert_eq!(nodes[0].node_type, NodeType::Heading { level: 1 });
        assert_eq!(nodes[1].node_type, NodeType::Paragraph);
        assert_eq!(nodes[2].node_type, NodeType::Paragraph);

        // Check CFI anchors are present
        for node in &nodes {
            assert!(node.cfi_anchor.is_some());
        }
    }
}
