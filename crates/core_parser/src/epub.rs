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

/// Cached OPF data parsed once and reused across metadata/toc/spine/cover calls.
struct OpfCache {
    /// Manifest items: id -> (href, media_type, properties).
    manifest: HashMap<String, (String, String, String)>,
    /// Spine item idrefs in order.
    spine_idrefs: Vec<String>,
    /// Metadata fields.
    title: String,
    authors: Vec<String>,
    language: Option<String>,
    publish_date: Option<String>,
    identifier: String,
    /// Cover item id from <meta name="cover" content="...">.
    cover_meta_id: Option<String>,
    /// TOC id from <spine toc="...">.
    toc_id: Option<String>,
}

/// EPUB parser that extracts metadata, TOC, and content from EPUB files.
pub struct EpubParser {
    /// Parsed ZIP archive (owns the only copy of EPUB bytes).
    archive: ZipArchive<Cursor<Vec<u8>>>,
    /// Path to the OPF file within the archive.
    opf_path: String,
    /// Base directory of the OPF file (for resolving relative paths).
    opf_base: String,
    /// Pre-computed SHA-256 hash of the original file.
    file_hash: String,
    /// Original file size in bytes.
    file_size: u64,
    /// Cached OPF parse results (lazily populated on first access).
    opf_cache: Option<OpfCache>,
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
    ///
    /// Computes the SHA-256 hash once at construction time, then transfers
    /// ownership of the data into the ZipArchive (no extra copy).
    pub fn new(data: Vec<u8>) -> Result<Self, ParseError> {
        if data.is_empty() {
            return Err(ParseError::EmptyContent);
        }

        // Compute hash and size before moving data into the archive.
        use sha2::{Digest, Sha256};
        let file_hash = format!("{:x}", Sha256::digest(&data));
        let file_size = data.len() as u64;

        // Move data directly into Cursor — no clone needed.
        let cursor = Cursor::new(data);
        let archive = ZipArchive::new(cursor)?;

        let mut parser = Self {
            archive,
            opf_path: String::new(),
            opf_base: String::new(),
            file_hash,
            file_size,
            opf_cache: None,
        };

        parser.locate_opf()?;
        Ok(parser)
    }

    /// Ensure the OPF cache is populated, parsing the OPF file only once.
    fn ensure_opf_cache(&mut self) -> Result<(), ParseError> {
        if self.opf_cache.is_some() {
            return Ok(());
        }

        let opf_data = self.read_file_from_archive(&self.opf_path.clone())?;
        let opf_str = String::from_utf8(opf_data)
            .map_err(|e| ParseError::InvalidEpub(format!("Invalid UTF-8 in OPF: {e}")))?;

        let mut title = String::new();
        let mut authors = Vec::new();
        let mut language = None;
        let mut publish_date = None;
        let mut identifier = String::new();
        let mut cover_meta_id = None;
        let mut toc_id = None;
        let mut manifest: HashMap<String, (String, String, String)> = HashMap::new();
        let mut spine_idrefs: Vec<String> = Vec::new();

        // Single-pass parse of the entire OPF file.
        let mut reader = Reader::from_str(&opf_str);
        reader.config_mut().trim_text(true);

        let mut buf = Vec::new();
        let mut current_tag = String::new();
        let mut in_metadata = false;
        let mut in_manifest = false;
        let mut in_spine = false;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let local_name = e.name().as_ref().to_vec();
                    let tag = String::from_utf8_lossy(&local_name).to_string();
                    let tag_short = tag.rsplit(':').next().unwrap_or(&tag).to_string();

                    match tag_short.as_str() {
                        "metadata" => in_metadata = true,
                        "manifest" => in_manifest = true,
                        "spine" => {
                            in_spine = true;
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"toc" {
                                    toc_id = Some(String::from_utf8_lossy(&attr.value).to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                    if in_metadata {
                        current_tag = tag_short;
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
                            if identifier.is_empty() {
                                identifier = text;
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::Empty(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    let name_short = name.rsplit(':').next().unwrap_or(&name);

                    if in_metadata && name_short == "meta" {
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
                            cover_meta_id = content_val;
                        }
                    } else if in_manifest && (name_short == "item") {
                        let mut item_id = String::new();
                        let mut href = String::new();
                        let mut media_type = String::new();
                        let mut properties = String::new();
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"id" => item_id = String::from_utf8_lossy(&attr.value).to_string(),
                                b"href" => href = String::from_utf8_lossy(&attr.value).to_string(),
                                b"media-type" => media_type = String::from_utf8_lossy(&attr.value).to_string(),
                                b"properties" => properties = String::from_utf8_lossy(&attr.value).to_string(),
                                _ => {}
                            }
                        }
                        manifest.insert(item_id, (href, media_type, properties));
                    } else if in_spine && (name_short == "itemref") {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"idref" {
                                spine_idrefs.push(String::from_utf8_lossy(&attr.value).to_string());
                            }
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let local_name = e.name().as_ref().to_vec();
                    let tag = String::from_utf8_lossy(&local_name).to_string();
                    let tag_short = tag.rsplit(':').next().unwrap_or(&tag);
                    match tag_short {
                        "metadata" => { in_metadata = false; }
                        "manifest" => { in_manifest = false; }
                        "spine" => { in_spine = false; }
                        _ => {}
                    }
                    current_tag.clear();
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(ParseError::InvalidXml(format!("OPF: {e}"))),
                _ => {}
            }
            buf.clear();
        }

        self.opf_cache = Some(OpfCache {
            manifest,
            spine_idrefs,
            title,
            authors,
            language,
            publish_date,
            identifier,
            cover_meta_id,
            toc_id,
        });

        Ok(())
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
    ///
    /// Uses cached OPF parse results — the OPF is only parsed once.
    pub fn parse_metadata(&mut self) -> Result<BookMetadata, ParseError> {
        self.ensure_opf_cache()?;
        let cache = self.opf_cache.as_ref().unwrap();

        let mut id = cache.identifier.clone();
        if id.is_empty() {
            id = self.file_hash.clone();
        }

        Ok(BookMetadata {
            id,
            title: cache.title.clone(),
            authors: cache.authors.clone(),
            language: cache.language.clone(),
            publish_date: cache.publish_date.clone(),
            cover_image_ref: cache.cover_meta_id.clone(),
            format: BookFormat::Epub,
            file_hash: Some(self.file_hash.clone()),
            file_size: Some(self.file_size),
        })
    }

    /// Parse the table of contents from NCX or NAV document.
    ///
    /// Uses cached OPF manifest to locate the TOC file.
    pub fn parse_toc(&mut self) -> Result<Vec<TocEntry>, ParseError> {
        self.ensure_opf_cache()?;
        let toc_path = self.find_toc_path_from_cache()?;
        let toc_data = self.read_file_from_archive(&toc_path)?;
        let toc_str = String::from_utf8(toc_data)
            .map_err(|e| ParseError::InvalidEpub(format!("Invalid UTF-8 in TOC: {e}")))?;

        if toc_path.ends_with(".ncx") {
            self.parse_ncx_toc(&toc_str)
        } else {
            self.parse_nav_toc(&toc_str)
        }
    }

    /// Find the TOC file path from the cached OPF manifest.
    fn find_toc_path_from_cache(&self) -> Result<String, ParseError> {
        let cache = self.opf_cache.as_ref().unwrap();

        // Try to find NAV document (EPUB 3)
        for (_id, (href, _media_type, properties)) in &cache.manifest {
            if properties.contains("nav") {
                return Ok(format!("{}{}", self.opf_base, href));
            }
        }

        // Fall back to NCX (EPUB 2)
        if let Some(ncx_id) = &cache.toc_id {
            if let Some((href, _media_type, _props)) = cache.manifest.get(ncx_id) {
                return Ok(format!("{}{}", self.opf_base, href));
            }
        }

        // Try to find any .ncx file
        for (_id, (href, media_type, _props)) in &cache.manifest {
            if media_type.contains("application/x-dtbncx+xml") {
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
    ///
    /// Uses cached OPF manifest and spine data.
    pub fn get_spine(&mut self) -> Result<Vec<String>, ParseError> {
        self.ensure_opf_cache()?;
        let cache = self.opf_cache.as_ref().unwrap();

        let paths: Vec<String> = cache
            .spine_idrefs
            .iter()
            .filter_map(|idref| {
                cache
                    .manifest
                    .get(idref)
                    .map(|(href, _media, _props)| format!("{}{}", self.opf_base, href))
            })
            .collect();

        Ok(paths)
    }

    /// Get the cover image data by resolving the cover item id from the manifest.
    ///
    /// Uses cached OPF manifest to locate the cover image.
    /// Returns the raw image bytes (JPEG, PNG, etc.) or an error if no cover is found.
    pub fn get_cover_image(&mut self, cover_id: &str) -> Result<Vec<u8>, ParseError> {
        self.ensure_opf_cache()?;
        let cache = self.opf_cache.as_ref().unwrap();

        // Find cover by item id or by cover-image property (EPUB 3)
        let cover_href = cache
            .manifest
            .iter()
            .find(|(id, (_href, _media, props))| {
                *id == cover_id || props.contains("cover-image")
            })
            .map(|(_id, (href, _media, _props))| href.clone());

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

    /// Read a resource file (image, CSS, font, etc.) from the EPUB archive.
    ///
    /// The `href` is typically the `src` attribute from an `<img>` tag in the
    /// chapter XHTML. It may be relative to the chapter file or to the OPF base.
    /// This method tries multiple resolution strategies:
    /// 1. Direct path lookup
    /// 2. Prepend OPF base directory
    /// 3. Search manifest by filename
    pub fn get_resource(&mut self, href: &str) -> Result<Vec<u8>, ParseError> {
        if href.is_empty() {
            return Err(ParseError::InvalidEpub("Empty resource href".to_string()));
        }

        // Strategy 1: Try the href as-is (absolute path within the archive)
        if let Ok(data) = self.read_file_from_archive(href) {
            return Ok(data);
        }

        // Strategy 2: Prepend OPF base directory
        let with_base = format!("{}{}", self.opf_base, href);
        if let Ok(data) = self.read_file_from_archive(&with_base) {
            return Ok(data);
        }

        // Strategy 3: Try resolving relative paths (e.g., "../images/foo.png")
        // by normalizing against the OPF base
        let normalized = normalize_path(&format!("{}{}", self.opf_base, href));
        if normalized != with_base {
            if let Ok(data) = self.read_file_from_archive(&normalized) {
                return Ok(data);
            }
        }

        // Strategy 4: Search manifest for matching filename
        self.ensure_opf_cache()?;
        let mut candidate_paths = Vec::new();
        if let Some(cache) = &self.opf_cache {
            let filename = href.rsplit('/').next().unwrap_or(href);
            for (_id, (manifest_href, _media, _props)) in &cache.manifest {
                let manifest_filename = manifest_href.rsplit('/').next().unwrap_or(manifest_href);
                if manifest_filename == filename {
                    candidate_paths.push(format!("{}{}", self.opf_base, manifest_href));
                }
            }
        }
        for path in &candidate_paths {
            if let Ok(data) = self.read_file_from_archive(path) {
                return Ok(data);
            }
        }

        Err(ParseError::InvalidEpub(format!(
            "Resource not found in archive: {}",
            href
        )))
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

    /// Resolve a TOC entry href to a spine index.
    ///
    /// Strips the fragment identifier (`#` and everything after) from the href,
    /// normalizes relative paths using `opf_base`, and matches against the spine.
    /// Returns the 0-based spine index on success, or -1 if no match is found.
    pub fn resolve_toc_href(&mut self, href: &str) -> Result<i32, ParseError> {
        self.ensure_opf_cache()?;

        // Strip fragment identifier
        let base_href = match href.find('#') {
            Some(pos) => &href[..pos],
            None => href,
        };

        if base_href.is_empty() {
            return Ok(-1);
        }

        // Build the full path by prepending opf_base if href is relative
        let full_href = if base_href.starts_with('/') {
            base_href.to_string()
        } else {
            format!("{}{}", self.opf_base, base_href)
        };

        // Get spine paths
        let spine = self.get_spine()?;

        // Try exact match first (most common case)
        for (i, spine_path) in spine.iter().enumerate() {
            if *spine_path == full_href {
                return Ok(i as i32);
            }
        }

        // Try matching without opf_base prefix (bare filename match)
        for (i, spine_path) in spine.iter().enumerate() {
            let spine_basename = spine_path
                .rfind('/')
                .map(|pos| &spine_path[pos + 1..])
                .unwrap_or(spine_path);
            if spine_basename == base_href {
                return Ok(i as i32);
            }
        }

        Ok(-1)
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

/// Normalize a path by resolving `.` and `..` segments.
fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
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

    // ─── resolve_toc_href Tests ──────────────────────────────────────────

    #[test]
    fn test_resolve_toc_href_exact_match() {
        let epub_data = create_test_epub();
        let mut parser = EpubParser::new(epub_data).unwrap();
        // "chapter01.xhtml" should resolve to spine index 0
        let idx = parser.resolve_toc_href("chapter01.xhtml").unwrap();
        assert_eq!(idx, 0);
    }

    #[test]
    fn test_resolve_toc_href_with_fragment() {
        let epub_data = create_test_epub();
        let mut parser = EpubParser::new(epub_data).unwrap();
        // "chapter01.xhtml#section2" should strip fragment and match index 0
        let idx = parser.resolve_toc_href("chapter01.xhtml#section2").unwrap();
        assert_eq!(idx, 0);
    }

    #[test]
    fn test_resolve_toc_href_no_match() {
        let epub_data = create_test_epub();
        let mut parser = EpubParser::new(epub_data).unwrap();
        let idx = parser.resolve_toc_href("nonexistent.xhtml").unwrap();
        assert_eq!(idx, -1);
    }

    #[test]
    fn test_resolve_toc_href_empty_string() {
        let epub_data = create_test_epub();
        let mut parser = EpubParser::new(epub_data).unwrap();
        let idx = parser.resolve_toc_href("").unwrap();
        assert_eq!(idx, -1);
    }

    #[test]
    fn test_resolve_toc_href_fragment_only() {
        let epub_data = create_test_epub();
        let mut parser = EpubParser::new(epub_data).unwrap();
        // "#section1" has empty base_href, should return -1
        let idx = parser.resolve_toc_href("#section1").unwrap();
        assert_eq!(idx, -1);
    }

    // ─── normalize_path Tests ────────────────────────────────────────────

    #[test]
    fn test_normalize_path_simple() {
        assert_eq!(normalize_path("OEBPS/images/photo.png"), "OEBPS/images/photo.png");
    }

    #[test]
    fn test_normalize_path_with_dot() {
        assert_eq!(normalize_path("OEBPS/./images/photo.png"), "OEBPS/images/photo.png");
    }

    #[test]
    fn test_normalize_path_with_dotdot() {
        assert_eq!(normalize_path("OEBPS/text/../images/photo.png"), "OEBPS/images/photo.png");
    }

    #[test]
    fn test_normalize_path_multiple_dotdot() {
        assert_eq!(
            normalize_path("OEBPS/text/sub/../../images/photo.png"),
            "OEBPS/images/photo.png"
        );
    }

    #[test]
    fn test_normalize_path_consecutive_slashes() {
        assert_eq!(normalize_path("OEBPS//images///photo.png"), "OEBPS/images/photo.png");
    }

    #[test]
    fn test_normalize_path_dotdot_at_root() {
        // ".." beyond root should be silently ignored (no panic)
        assert_eq!(normalize_path("../images/photo.png"), "images/photo.png");
    }

    #[test]
    fn test_normalize_path_only_dotdot() {
        assert_eq!(normalize_path(".."), "");
    }

    #[test]
    fn test_normalize_path_empty() {
        assert_eq!(normalize_path(""), "");
    }

    #[test]
    fn test_normalize_path_only_dots() {
        assert_eq!(normalize_path("./././."), "");
    }

    #[test]
    fn test_normalize_path_trailing_slash() {
        assert_eq!(normalize_path("OEBPS/images/"), "OEBPS/images");
    }

    #[test]
    fn test_normalize_path_complex() {
        assert_eq!(
            normalize_path("OEBPS/text/chapter/../sub/./../../images/cover.jpg"),
            "OEBPS/images/cover.jpg"
        );
    }

    // ─── get_resource Tests ──────────────────────────────────────────────

    /// Helper to create a test EPUB with image resources at various paths
    /// for testing all 4 resource resolution strategies.
    fn create_test_epub_with_images() -> Vec<u8> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let buf = Vec::new();
        let cursor = Cursor::new(buf);
        let mut zip = ZipWriter::new(cursor);
        let options = SimpleFileOptions::default();

        // Fake 1x1 PNG data (minimal valid PNG)
        let fake_png: &[u8] = b"FAKE_PNG_DATA_FOR_TESTING";
        let fake_jpg: &[u8] = b"FAKE_JPG_DATA_FOR_TESTING";

        // mimetype
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

        // content.opf with image items in manifest
        zip.start_file("OEBPS/content.opf", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Image Test Book</dc:title>
    <dc:identifier id="uid">test-img-001</dc:identifier>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="chap01" href="text/chapter01.xhtml" media-type="application/xhtml+xml"/>
    <item id="img_direct" href="images/direct.png" media-type="image/png"/>
    <item id="img_cover" href="images/cover.jpg" media-type="image/jpeg"/>
    <item id="img_unique" href="images/unique_name.png" media-type="image/png"/>
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
<head><title>Nav</title></head>
<body>
  <nav epub:type="toc"><ol><li><a href="text/chapter01.xhtml">Ch 1</a></li></ol></nav>
</body>
</html>"#,
        )
        .unwrap();

        // chapter01.xhtml
        zip.start_file("OEBPS/text/chapter01.xhtml", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter 1</title></head>
<body>
  <h1>Chapter 1</h1>
  <p>Text with image:</p>
  <img src="../images/direct.png" alt="test"/>
</body>
</html>"#,
        )
        .unwrap();

        // Image files at various paths
        // Strategy 1: Direct path — "OEBPS/images/direct.png"
        zip.start_file("OEBPS/images/direct.png", options).unwrap();
        zip.write_all(fake_png).unwrap();

        // Strategy 2: OPF base + href — "OEBPS/images/cover.jpg"
        zip.start_file("OEBPS/images/cover.jpg", options).unwrap();
        zip.write_all(fake_jpg).unwrap();

        // Strategy 4: Unique filename match — "OEBPS/images/unique_name.png"
        zip.start_file("OEBPS/images/unique_name.png", options).unwrap();
        zip.write_all(fake_png).unwrap();

        let cursor = zip.finish().unwrap();
        cursor.into_inner()
    }

    #[test]
    fn test_get_resource_strategy1_direct_path() {
        // Strategy 1: href matches an exact path in the archive
        let epub_data = create_test_epub_with_images();
        let mut parser = EpubParser::new(epub_data).unwrap();
        let data = parser.get_resource("OEBPS/images/direct.png").unwrap();
        assert_eq!(data, b"FAKE_PNG_DATA_FOR_TESTING");
    }

    #[test]
    fn test_get_resource_strategy2_opf_base_prefix() {
        // Strategy 2: href is relative, prepend OPF base ("OEBPS/")
        let epub_data = create_test_epub_with_images();
        let mut parser = EpubParser::new(epub_data).unwrap();
        let data = parser.get_resource("images/cover.jpg").unwrap();
        assert_eq!(data, b"FAKE_JPG_DATA_FOR_TESTING");
    }

    #[test]
    fn test_get_resource_strategy3_relative_path_normalization() {
        // Strategy 3: href contains "../" relative path that needs normalization
        // From chapter at "OEBPS/text/chapter01.xhtml", image src="../images/direct.png"
        // Normalized: "OEBPS/" + "text/../images/direct.png" -> "OEBPS/images/direct.png"
        let epub_data = create_test_epub_with_images();
        let mut parser = EpubParser::new(epub_data).unwrap();
        // Simulate what the renderer would pass: the raw src from <img> tag
        // which is relative to the chapter file's directory
        let data = parser.get_resource("text/../images/direct.png").unwrap();
        assert_eq!(data, b"FAKE_PNG_DATA_FOR_TESTING");
    }

    #[test]
    fn test_get_resource_strategy4_filename_match() {
        // Strategy 4: Only the filename is provided, matched via manifest
        let epub_data = create_test_epub_with_images();
        let mut parser = EpubParser::new(epub_data).unwrap();
        let data = parser.get_resource("unique_name.png").unwrap();
        assert_eq!(data, b"FAKE_PNG_DATA_FOR_TESTING");
    }

    #[test]
    fn test_get_resource_not_found() {
        let epub_data = create_test_epub_with_images();
        let mut parser = EpubParser::new(epub_data).unwrap();
        let result = parser.get_resource("nonexistent_image.png");
        assert!(result.is_err());
        match result.unwrap_err() {
            ParseError::InvalidEpub(msg) => {
                assert!(msg.contains("Resource not found"));
            }
            other => panic!("Expected InvalidEpub, got: {:?}", other),
        }
    }

    #[test]
    fn test_get_resource_empty_href() {
        let epub_data = create_test_epub_with_images();
        let mut parser = EpubParser::new(epub_data).unwrap();
        let result = parser.get_resource("");
        assert!(result.is_err());
        match result.unwrap_err() {
            ParseError::InvalidEpub(msg) => {
                assert!(msg.contains("Empty resource href"));
            }
            other => panic!("Expected InvalidEpub, got: {:?}", other),
        }
    }

    #[test]
    fn test_get_resource_multiple_strategies_fallthrough() {
        // Verify that when Strategy 1 fails, it falls through to Strategy 2
        let epub_data = create_test_epub_with_images();
        let mut parser = EpubParser::new(epub_data).unwrap();
        // "images/cover.jpg" won't match directly (no "OEBPS/" prefix),
        // but Strategy 2 prepends "OEBPS/" and finds it
        let data = parser.get_resource("images/cover.jpg").unwrap();
        assert_eq!(data, b"FAKE_JPG_DATA_FOR_TESTING");
    }
}
