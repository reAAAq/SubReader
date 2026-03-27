//! C-ABI bridge layer for native platforms (Flutter, iOS, Android, etc.).
//!
//! All exported functions use fixed-width scalar types only (i32, u32, u64, *const u8).
//! Strings are passed as pointer-length pairs (*const u8 + u32).
//! All functions use `catch_unwind` to prevent panics from crossing the FFI boundary.

use std::panic::catch_unwind;
use std::slice;
use std::sync::Mutex;

use core_parser::EpubParser;
use core_parser::TxtParser;
use core_state::StateManager;
use shared_types::{Annotation, Bookmark};

// ─── FFI Error Codes ─────────────────────────────────────────────────────────

pub const FFI_OK: i32 = 0;
pub const FFI_ERR_NULL_PTR: i32 = -1;
pub const FFI_ERR_INVALID_UTF8: i32 = -2;
pub const FFI_ERR_PARSE_FAILED: i32 = -3;
pub const FFI_ERR_STORAGE: i32 = -4;
pub const FFI_ERR_NOT_FOUND: i32 = -5;
pub const FFI_ERR_ALREADY_INIT: i32 = -6;
pub const FFI_ERR_NOT_INIT: i32 = -7;
pub const FFI_ERR_PANIC: i32 = -98;
pub const FFI_ERR_UNKNOWN: i32 = -99;

// ─── Global Engine State ─────────────────────────────────────────────────────

struct EngineState {
    state_manager: StateManager,
    /// Currently opened book parser (if any).
    current_book: Option<EpubParser>,
    /// Buffer for returning string data to the caller.
    return_buffer: String,
    /// Buffer for returning binary data (e.g. cover images) to the caller.
    binary_buffer: Vec<u8>,
}

// NOTE: We use Mutex instead of RwLock because EngineState contains
// rusqlite::Connection (via StateManager), which uses RefCell internally
// and does not implement Sync. Mutex<T> only requires T: Send, while
// RwLock<T> requires T: Send + Sync.
static ENGINE: Mutex<Option<EngineState>> = Mutex::new(None);

// ─── Helper Functions ────────────────────────────────────────────────────────

/// Convert a pointer-length pair to a Rust &str.
///
/// # Safety
/// Caller must ensure ptr is valid for `len` bytes.
unsafe fn ptr_to_str<'a>(ptr: *const u8, len: u32) -> Result<&'a str, i32> {
    if ptr.is_null() {
        return Err(FFI_ERR_NULL_PTR);
    }
    let bytes = slice::from_raw_parts(ptr, len as usize);
    std::str::from_utf8(bytes).map_err(|_| FFI_ERR_INVALID_UTF8)
}

/// Copy a string into the engine's return buffer and return its pointer and length.
fn set_return_buffer(engine: &mut EngineState, s: String) -> (*const u8, u32) {
    engine.return_buffer = s;
    (
        engine.return_buffer.as_ptr(),
        engine.return_buffer.len() as u32,
    )
}

// ─── Engine Lifecycle ────────────────────────────────────────────────────────

/// Initialize the reader engine with a database path.
///
/// # Safety
/// `db_path_ptr` must point to valid UTF-8 bytes of length `db_path_len`.
#[no_mangle]
pub unsafe extern "C" fn reader_engine_init(
    db_path_ptr: *const u8,
    db_path_len: u32,
    device_id_ptr: *const u8,
    device_id_len: u32,
) -> i32 {
    catch_unwind(|| {
        let db_path = match unsafe { ptr_to_str(db_path_ptr, db_path_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };
        let device_id = match unsafe { ptr_to_str(device_id_ptr, device_id_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };

        let mut guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        if guard.is_some() {
            return FFI_ERR_ALREADY_INIT;
        }

        match StateManager::new(db_path, device_id) {
            Ok(sm) => {
                *guard = Some(EngineState {
                    state_manager: sm,
                    current_book: None,
                    return_buffer: String::new(),
                    binary_buffer: Vec::new(),
                });
                FFI_OK
            }
            Err(_) => FFI_ERR_STORAGE,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Destroy the reader engine and free resources.
///
/// # Safety
/// Must be called after `reader_engine_init`.
#[no_mangle]
pub unsafe extern "C" fn reader_engine_destroy() -> i32 {
    catch_unwind(|| {
        let mut guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };
        *guard = None;
        FFI_OK
    })
    .unwrap_or(FFI_ERR_PANIC)
}

// ─── Book Operations ─────────────────────────────────────────────────────────

/// Open an EPUB book from raw bytes.
///
/// # Safety
/// `data_ptr` must point to valid bytes of length `data_len`.
#[no_mangle]
pub unsafe extern "C" fn reader_open_book(data_ptr: *const u8, data_len: u32) -> i32 {
    catch_unwind(|| {
        if data_ptr.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let data = unsafe { slice::from_raw_parts(data_ptr, data_len as usize) }.to_vec();

        let mut guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_mut() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        match EpubParser::new(data) {
            Ok(parser) => {
                engine.current_book = Some(parser);
                FFI_OK
            }
            Err(_) => FFI_ERR_PARSE_FAILED,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Close the currently opened book.
///
/// # Safety
/// Must be called after `reader_open_book`.
#[no_mangle]
pub unsafe extern "C" fn reader_close_book() -> i32 {
    catch_unwind(|| {
        let mut guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_mut() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        engine.current_book = None;
        FFI_OK
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Get book metadata as JSON string.
///
/// # Safety
/// `out_ptr` and `out_len` must be valid pointers.
#[no_mangle]
pub unsafe extern "C" fn reader_get_metadata(out_ptr: *mut *const u8, out_len: *mut u32) -> i32 {
    catch_unwind(|| {
        if out_ptr.is_null() || out_len.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let mut guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_mut() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        let parser = match engine.current_book.as_mut() {
            Some(p) => p,
            None => return FFI_ERR_NOT_FOUND,
        };

        match parser.parse_metadata() {
            Ok(metadata) => match serde_json::to_string(&metadata) {
                Ok(json) => {
                    let (ptr, len) = set_return_buffer(engine, json);
                    unsafe {
                        *out_ptr = ptr;
                        *out_len = len;
                    }
                    FFI_OK
                }
                Err(_) => FFI_ERR_UNKNOWN,
            },
            Err(_) => FFI_ERR_PARSE_FAILED,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Get chapter content as JSON DOM tree.
///
/// # Safety
/// All pointer parameters must be valid.
#[no_mangle]
pub unsafe extern "C" fn reader_get_chapter_content(
    path_ptr: *const u8,
    path_len: u32,
    out_ptr: *mut *const u8,
    out_len: *mut u32,
) -> i32 {
    catch_unwind(|| {
        let path = match unsafe { ptr_to_str(path_ptr, path_len) } {
            Ok(s) => s.to_string(),
            Err(code) => return code,
        };

        if out_ptr.is_null() || out_len.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let mut guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_mut() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        let parser = match engine.current_book.as_mut() {
            Some(p) => p,
            None => return FFI_ERR_NOT_FOUND,
        };

        match parser.parse_chapter(&path) {
            Ok(nodes) => match serde_json::to_string(&nodes) {
                Ok(json) => {
                    let (ptr, len) = set_return_buffer(engine, json);
                    unsafe {
                        *out_ptr = ptr;
                        *out_len = len;
                    }
                    FFI_OK
                }
                Err(_) => FFI_ERR_UNKNOWN,
            },
            Err(_) => FFI_ERR_PARSE_FAILED,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

// ─── TOC / Spine / Cover Operations ─────────────────────────────────────────

/// Get the table of contents as JSON.
///
/// # Safety
/// `out_ptr` and `out_len` must be valid pointers.
#[no_mangle]
pub unsafe extern "C" fn reader_get_toc(out_ptr: *mut *const u8, out_len: *mut u32) -> i32 {
    catch_unwind(|| {
        if out_ptr.is_null() || out_len.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let mut guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_mut() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        let parser = match engine.current_book.as_mut() {
            Some(p) => p,
            None => return FFI_ERR_NOT_FOUND,
        };

        match parser.parse_toc() {
            Ok(toc) => match serde_json::to_string(&toc) {
                Ok(json) => {
                    let (ptr, len) = set_return_buffer(engine, json);
                    unsafe {
                        *out_ptr = ptr;
                        *out_len = len;
                    }
                    FFI_OK
                }
                Err(_) => FFI_ERR_UNKNOWN,
            },
            Err(_) => FFI_ERR_PARSE_FAILED,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Get the spine (ordered content document paths) as JSON.
///
/// # Safety
/// `out_ptr` and `out_len` must be valid pointers.
#[no_mangle]
pub unsafe extern "C" fn reader_get_spine(out_ptr: *mut *const u8, out_len: *mut u32) -> i32 {
    catch_unwind(|| {
        if out_ptr.is_null() || out_len.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let mut guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_mut() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        let parser = match engine.current_book.as_mut() {
            Some(p) => p,
            None => return FFI_ERR_NOT_FOUND,
        };

        match parser.get_spine() {
            Ok(spine) => match serde_json::to_string(&spine) {
                Ok(json) => {
                    let (ptr, len) = set_return_buffer(engine, json);
                    unsafe {
                        *out_ptr = ptr;
                        *out_len = len;
                    }
                    FFI_OK
                }
                Err(_) => FFI_ERR_UNKNOWN,
            },
            Err(_) => FFI_ERR_PARSE_FAILED,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Get the cover image as raw bytes.
///
/// The cover_id is the manifest item id from BookMetadata.cover_image_ref.
///
/// # Safety
/// All pointer parameters must be valid.
#[no_mangle]
pub unsafe extern "C" fn reader_get_cover_image(
    cover_id_ptr: *const u8,
    cover_id_len: u32,
    out_ptr: *mut *const u8,
    out_len: *mut u32,
) -> i32 {
    catch_unwind(|| {
        let cover_id = match unsafe { ptr_to_str(cover_id_ptr, cover_id_len) } {
            Ok(s) => s.to_string(),
            Err(code) => return code,
        };

        if out_ptr.is_null() || out_len.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let mut guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_mut() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        let parser = match engine.current_book.as_mut() {
            Some(p) => p,
            None => return FFI_ERR_NOT_FOUND,
        };

        match parser.get_cover_image(&cover_id) {
            Ok(image_data) => {
                // Store binary data in a dedicated buffer to avoid UB.
                let len = image_data.len() as u32;
                engine.binary_buffer = image_data;
                let ptr = engine.binary_buffer.as_ptr();
                unsafe {
                    *out_ptr = ptr;
                    *out_len = len;
                }
                FFI_OK
            }
            Err(_) => FFI_ERR_NOT_FOUND,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Resolve a TOC entry href to a spine index.
///
/// Returns the 0-based spine index (>= 0) on success, -1 if no match,
/// or a negative FFI error code on failure.
///
/// # Safety
/// `href_ptr` must point to valid UTF-8 bytes of length `href_len`.
#[no_mangle]
pub unsafe extern "C" fn reader_resolve_toc_href(href_ptr: *const u8, href_len: u32) -> i32 {
    catch_unwind(|| {
        let href = match unsafe { ptr_to_str(href_ptr, href_len) } {
            Ok(s) => s.to_string(),
            Err(code) => return code,
        };

        let mut guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_mut() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        let parser = match engine.current_book.as_mut() {
            Some(p) => p,
            None => return FFI_ERR_NOT_FOUND,
        };

        match parser.resolve_toc_href(&href) {
            Ok(idx) => idx,
            Err(_) => FFI_ERR_PARSE_FAILED,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

// ─── TXT Operations ──────────────────────────────────────────────────────────

/// Parse a TXT file from raw bytes with chapter splitting.
/// Returns JSON with { "encoding": "...", "had_replacements": bool, "chapters": [...] }.
/// Each chapter has { "title": "...", "nodes": [...] }.
///
/// This is a stateless operation — no need to call reader_open_book first.
///
/// # Safety
/// `data_ptr` must point to valid bytes of length `data_len`.
/// `out_ptr` and `out_len` must be valid pointers.
#[no_mangle]
pub unsafe extern "C" fn reader_parse_txt(
    data_ptr: *const u8,
    data_len: u32,
    out_ptr: *mut *const u8,
    out_len: *mut u32,
) -> i32 {
    catch_unwind(|| {
        if data_ptr.is_null() || out_ptr.is_null() || out_len.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let data = unsafe { slice::from_raw_parts(data_ptr, data_len as usize) };

        let result = match TxtParser::parse_with_chapters(data) {
            Ok(r) => r,
            Err(_) => return FFI_ERR_PARSE_FAILED,
        };

        let json = match serde_json::to_string(&result) {
            Ok(j) => j,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let mut guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_mut() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        let (ptr, len) = set_return_buffer(engine, json);
        unsafe {
            *out_ptr = ptr;
            *out_len = len;
        }
        FFI_OK
    })
    .unwrap_or(FFI_ERR_PANIC)
}

// ─── Progress Operations ─────────────────────────────────────────────────────

/// Get reading progress as JSON.
///
/// # Safety
/// All pointer parameters must be valid.
#[no_mangle]
pub unsafe extern "C" fn reader_get_progress(
    book_id_ptr: *const u8,
    book_id_len: u32,
    out_ptr: *mut *const u8,
    out_len: *mut u32,
) -> i32 {
    catch_unwind(|| {
        let book_id = match unsafe { ptr_to_str(book_id_ptr, book_id_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };

        if out_ptr.is_null() || out_len.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let mut guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_mut() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        match engine.state_manager.get_progress(book_id) {
            Ok(Some(progress)) => match serde_json::to_string(&progress) {
                Ok(json) => {
                    let (ptr, len) = set_return_buffer(engine, json);
                    unsafe {
                        *out_ptr = ptr;
                        *out_len = len;
                    }
                    FFI_OK
                }
                Err(_) => FFI_ERR_UNKNOWN,
            },
            Ok(None) => FFI_ERR_NOT_FOUND,
            Err(_) => FFI_ERR_STORAGE,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Update reading progress.
///
/// # Safety
/// All pointer parameters must be valid.
#[no_mangle]
pub unsafe extern "C" fn reader_update_progress(
    book_id_ptr: *const u8,
    book_id_len: u32,
    cfi_ptr: *const u8,
    cfi_len: u32,
    percentage: f64,
    hlc_ts: u64,
) -> i32 {
    catch_unwind(|| {
        let book_id = match unsafe { ptr_to_str(book_id_ptr, book_id_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };
        let cfi = match unsafe { ptr_to_str(cfi_ptr, cfi_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };

        let guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_ref() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        match engine
            .state_manager
            .update_progress(book_id, cfi, percentage, hlc_ts)
        {
            Ok(()) => FFI_OK,
            Err(_) => FFI_ERR_STORAGE,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

// ─── Bookmark Operations ─────────────────────────────────────────────────────

/// Add a bookmark (JSON input).
///
/// # Safety
/// All pointer parameters must be valid.
#[no_mangle]
pub unsafe extern "C" fn reader_add_bookmark(json_ptr: *const u8, json_len: u32) -> i32 {
    catch_unwind(|| {
        let json_str = match unsafe { ptr_to_str(json_ptr, json_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };

        let bookmark: Bookmark = match serde_json::from_str(json_str) {
            Ok(b) => b,
            Err(_) => return FFI_ERR_INVALID_UTF8,
        };

        let guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_ref() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        match engine.state_manager.add_bookmark(&bookmark) {
            Ok(()) => FFI_OK,
            Err(_) => FFI_ERR_STORAGE,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Delete a bookmark by ID.
///
/// # Safety
/// All pointer parameters must be valid.
#[no_mangle]
pub unsafe extern "C" fn reader_delete_bookmark(
    id_ptr: *const u8,
    id_len: u32,
    hlc_ts: u64,
) -> i32 {
    catch_unwind(|| {
        let id = match unsafe { ptr_to_str(id_ptr, id_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };

        let guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_ref() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        match engine.state_manager.delete_bookmark(id, hlc_ts) {
            Ok(true) => FFI_OK,
            Ok(false) => FFI_ERR_NOT_FOUND,
            Err(_) => FFI_ERR_STORAGE,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// List bookmarks for a book as JSON.
///
/// # Safety
/// All pointer parameters must be valid.
#[no_mangle]
pub unsafe extern "C" fn reader_list_bookmarks(
    book_id_ptr: *const u8,
    book_id_len: u32,
    out_ptr: *mut *const u8,
    out_len: *mut u32,
) -> i32 {
    catch_unwind(|| {
        let book_id = match unsafe { ptr_to_str(book_id_ptr, book_id_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };

        if out_ptr.is_null() || out_len.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let mut guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_mut() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        match engine.state_manager.list_bookmarks(book_id) {
            Ok(bookmarks) => match serde_json::to_string(&bookmarks) {
                Ok(json) => {
                    let (ptr, len) = set_return_buffer(engine, json);
                    unsafe {
                        *out_ptr = ptr;
                        *out_len = len;
                    }
                    FFI_OK
                }
                Err(_) => FFI_ERR_UNKNOWN,
            },
            Err(_) => FFI_ERR_STORAGE,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

// ─── Annotation Operations ───────────────────────────────────────────────────

/// Add an annotation (JSON input).
///
/// # Safety
/// All pointer parameters must be valid.
#[no_mangle]
pub unsafe extern "C" fn reader_add_annotation(json_ptr: *const u8, json_len: u32) -> i32 {
    catch_unwind(|| {
        let json_str = match unsafe { ptr_to_str(json_ptr, json_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };

        let annotation: Annotation = match serde_json::from_str(json_str) {
            Ok(a) => a,
            Err(_) => return FFI_ERR_INVALID_UTF8,
        };

        let guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_ref() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        match engine.state_manager.add_annotation(&annotation) {
            Ok(()) => FFI_OK,
            Err(_) => FFI_ERR_STORAGE,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Delete an annotation by ID.
///
/// # Safety
/// All pointer parameters must be valid.
#[no_mangle]
pub unsafe extern "C" fn reader_delete_annotation(
    id_ptr: *const u8,
    id_len: u32,
    hlc_ts: u64,
) -> i32 {
    catch_unwind(|| {
        let id = match unsafe { ptr_to_str(id_ptr, id_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };

        let guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_ref() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        match engine.state_manager.delete_annotation(id, hlc_ts) {
            Ok(true) => FFI_OK,
            Ok(false) => FFI_ERR_NOT_FOUND,
            Err(_) => FFI_ERR_STORAGE,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// List annotations for a book as JSON.
///
/// # Safety
/// All pointer parameters must be valid.
#[no_mangle]
pub unsafe extern "C" fn reader_list_annotations(
    book_id_ptr: *const u8,
    book_id_len: u32,
    out_ptr: *mut *const u8,
    out_len: *mut u32,
) -> i32 {
    catch_unwind(|| {
        let book_id = match unsafe { ptr_to_str(book_id_ptr, book_id_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };

        if out_ptr.is_null() || out_len.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let mut guard = match ENGINE.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        let engine = match guard.as_mut() {
            Some(e) => e,
            None => return FFI_ERR_NOT_INIT,
        };

        match engine.state_manager.list_annotations(book_id) {
            Ok(annotations) => match serde_json::to_string(&annotations) {
                Ok(json) => {
                    let (ptr, len) = set_return_buffer(engine, json);
                    unsafe {
                        *out_ptr = ptr;
                        *out_len = len;
                    }
                    FFI_OK
                }
                Err(_) => FFI_ERR_UNKNOWN,
            },
            Err(_) => FFI_ERR_STORAGE,
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ffi_c tests share a global ENGINE, so we must serialize them.
    // Use a global test mutex to prevent concurrent access.
    // We handle PoisonError to prevent cascading failures.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Helper: acquire test lock, handling poison errors.
    fn acquire_lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Helper: reset engine state between tests.
    fn reset_engine() {
        let mut guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
        *guard = None;
    }

    /// Helper: create a temp directory and return the db path string.
    fn temp_db_path() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        (dir, path.to_str().unwrap().to_string())
    }

    /// Helper: initialize engine with a database path.
    fn init_engine_with_path(db_path: &str) {
        let device_id = b"test-device";
        let result = unsafe {
            reader_engine_init(
                db_path.as_ptr(),
                db_path.len() as u32,
                device_id.as_ptr(),
                device_id.len() as u32,
            )
        };
        assert_eq!(result, FFI_OK);
    }

    /// Helper: register a test book in the engine's state manager.
    fn register_test_book(book_id: &str) {
        let mut guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
        let engine = guard.as_mut().unwrap();
        engine
            .state_manager
            .register_book(book_id, "Test Book", "Author", "epub", None, None)
            .unwrap();
    }

    #[test]
    fn test_engine_not_init() {
        let _lock = acquire_lock();
        reset_engine();
        let result = unsafe { reader_close_book() };
        assert_eq!(result, FFI_ERR_NOT_INIT);
    }

    #[test]
    fn test_engine_destroy_without_init() {
        let _lock = acquire_lock();
        reset_engine();
        let result = unsafe { reader_engine_destroy() };
        // Should succeed even if not initialized
        assert_eq!(result, FFI_OK);
    }

    #[test]
    fn test_null_pointer_handling() {
        let _lock = acquire_lock();
        reset_engine();
        let result = unsafe { reader_engine_init(std::ptr::null(), 0, std::ptr::null(), 0) };
        assert_eq!(result, FFI_ERR_NULL_PTR);
    }

    #[test]
    fn test_open_book_not_init() {
        let _lock = acquire_lock();
        reset_engine();
        let data = b"not an epub";
        let result = unsafe { reader_open_book(data.as_ptr(), data.len() as u32) };
        assert_eq!(result, FFI_ERR_NOT_INIT);
    }

    #[test]
    fn test_error_codes_values() {
        assert_eq!(FFI_OK, 0);
        assert_eq!(FFI_ERR_NULL_PTR, -1);
        assert_eq!(FFI_ERR_INVALID_UTF8, -2);
        assert_eq!(FFI_ERR_PARSE_FAILED, -3);
        assert_eq!(FFI_ERR_STORAGE, -4);
        assert_eq!(FFI_ERR_NOT_FOUND, -5);
        assert_eq!(FFI_ERR_ALREADY_INIT, -6);
        assert_eq!(FFI_ERR_NOT_INIT, -7);
        assert_eq!(FFI_ERR_PANIC, -98);
        assert_eq!(FFI_ERR_UNKNOWN, -99);
    }

    // ─── Engine Lifecycle Tests ──────────────────────────────────────────────

    #[test]
    fn test_engine_init_success() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);
        let result = unsafe { reader_engine_destroy() };
        assert_eq!(result, FFI_OK);
    }

    #[test]
    fn test_engine_double_init_returns_already_init() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);
        // Second init should fail
        let device_id = b"test-device";
        let result = unsafe {
            reader_engine_init(
                db_path.as_ptr(),
                db_path.len() as u32,
                device_id.as_ptr(),
                device_id.len() as u32,
            )
        };
        assert_eq!(result, FFI_ERR_ALREADY_INIT);
        unsafe { reader_engine_destroy() };
    }

    #[test]
    fn test_engine_init_destroy_reinit() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);
        let result = unsafe { reader_engine_destroy() };
        assert_eq!(result, FFI_OK);
        // Re-init should succeed
        init_engine_with_path(&db_path);
        let result = unsafe { reader_engine_destroy() };
        assert_eq!(result, FFI_OK);
    }

    // ─── Invalid UTF-8 Input Tests ───────────────────────────────────────────

    #[test]
    fn test_invalid_utf8_db_path() {
        let _lock = acquire_lock();
        reset_engine();
        let invalid_bytes: &[u8] = &[0xFF, 0xFE, 0xFD];
        let device_id = b"test-device";
        let result = unsafe {
            reader_engine_init(
                invalid_bytes.as_ptr(),
                invalid_bytes.len() as u32,
                device_id.as_ptr(),
                device_id.len() as u32,
            )
        };
        assert_eq!(result, FFI_ERR_INVALID_UTF8);
    }

    #[test]
    fn test_invalid_utf8_device_id() {
        let _lock = acquire_lock();
        reset_engine();
        let db_path = b"/tmp/test.db";
        let invalid_bytes: &[u8] = &[0xFF, 0xFE, 0xFD];
        let result = unsafe {
            reader_engine_init(
                db_path.as_ptr(),
                db_path.len() as u32,
                invalid_bytes.as_ptr(),
                invalid_bytes.len() as u32,
            )
        };
        assert_eq!(result, FFI_ERR_INVALID_UTF8);
    }

    // ─── Open Book Tests ─────────────────────────────────────────────────────

    #[test]
    fn test_open_book_null_ptr() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);
        let result = unsafe { reader_open_book(std::ptr::null(), 0) };
        assert_eq!(result, FFI_ERR_NULL_PTR);
        unsafe { reader_engine_destroy() };
    }

    #[test]
    fn test_open_book_invalid_data() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);
        let data = b"this is not a valid epub file";
        let result = unsafe { reader_open_book(data.as_ptr(), data.len() as u32) };
        assert_eq!(result, FFI_ERR_PARSE_FAILED);
        unsafe { reader_engine_destroy() };
    }

    // ─── Get Metadata Without Book Tests ─────────────────────────────────────

    #[test]
    fn test_get_metadata_no_book_open() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);
        let mut out_ptr: *const u8 = std::ptr::null();
        let mut out_len: u32 = 0;
        let result = unsafe { reader_get_metadata(&mut out_ptr, &mut out_len) };
        assert_eq!(result, FFI_ERR_NOT_FOUND);
        unsafe { reader_engine_destroy() };
    }

    #[test]
    fn test_get_metadata_null_out_ptrs() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);
        let result = unsafe { reader_get_metadata(std::ptr::null_mut(), std::ptr::null_mut()) };
        assert_eq!(result, FFI_ERR_NULL_PTR);
        unsafe { reader_engine_destroy() };
    }

    // ─── Get Chapter Content Without Book Tests ──────────────────────────────

    #[test]
    fn test_get_chapter_content_no_book() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);
        let path = b"chapter1.xhtml";
        let mut out_ptr: *const u8 = std::ptr::null();
        let mut out_len: u32 = 0;
        let result = unsafe {
            reader_get_chapter_content(
                path.as_ptr(),
                path.len() as u32,
                &mut out_ptr,
                &mut out_len,
            )
        };
        assert_eq!(result, FFI_ERR_NOT_FOUND);
        unsafe { reader_engine_destroy() };
    }

    #[test]
    fn test_get_chapter_content_null_out_ptrs() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);
        let path = b"chapter1.xhtml";
        let result = unsafe {
            reader_get_chapter_content(
                path.as_ptr(),
                path.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(result, FFI_ERR_NULL_PTR);
        unsafe { reader_engine_destroy() };
    }

    // ─── Progress Operations Tests ───────────────────────────────────────────

    #[test]
    fn test_progress_not_init() {
        let _lock = acquire_lock();
        reset_engine();
        let book_id = b"book-1";
        let mut out_ptr: *const u8 = std::ptr::null();
        let mut out_len: u32 = 0;
        let result = unsafe {
            reader_get_progress(
                book_id.as_ptr(),
                book_id.len() as u32,
                &mut out_ptr,
                &mut out_len,
            )
        };
        assert_eq!(result, FFI_ERR_NOT_INIT);
    }

    #[test]
    fn test_update_and_get_progress() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);
        register_test_book("book-1");

        let book_id = b"book-1";
        let cfi = b"/6/4!/4/2:0";

        // Update progress
        let result = unsafe {
            reader_update_progress(
                book_id.as_ptr(),
                book_id.len() as u32,
                cfi.as_ptr(),
                cfi.len() as u32,
                42.5,
                1000,
            )
        };
        assert_eq!(result, FFI_OK);

        // Get progress
        let mut out_ptr: *const u8 = std::ptr::null();
        let mut out_len: u32 = 0;
        let result = unsafe {
            reader_get_progress(
                book_id.as_ptr(),
                book_id.len() as u32,
                &mut out_ptr,
                &mut out_len,
            )
        };
        assert_eq!(result, FFI_OK);
        assert!(!out_ptr.is_null());
        assert!(out_len > 0);

        // Verify JSON content
        let json_str =
            unsafe { std::str::from_utf8(slice::from_raw_parts(out_ptr, out_len as usize)) }
                .unwrap();
        assert!(json_str.contains("42.5"));
        assert!(json_str.contains("/6/4!/4/2:0"));

        unsafe { reader_engine_destroy() };
    }

    #[test]
    fn test_get_progress_not_found() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);

        let book_id = b"nonexistent-book";
        let mut out_ptr: *const u8 = std::ptr::null();
        let mut out_len: u32 = 0;
        let result = unsafe {
            reader_get_progress(
                book_id.as_ptr(),
                book_id.len() as u32,
                &mut out_ptr,
                &mut out_len,
            )
        };
        assert_eq!(result, FFI_ERR_NOT_FOUND);

        unsafe { reader_engine_destroy() };
    }

    // ─── Bookmark Operations Tests ───────────────────────────────────────────

    #[test]
    fn test_bookmark_crud_via_ffi() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);
        register_test_book("book-1");
        let bookmark_json = r#"{"id":"bm-1","book_id":"book-1","cfi_position":"/6/4!/4/2:0","title":"Chapter 1","created_at":1000}"#;
        let result = unsafe {
            reader_add_bookmark(
                bookmark_json.as_ptr(),
                bookmark_json.len() as u32,
            )
        };
        assert_eq!(result, FFI_OK);

        // List bookmarks
        let book_id = b"book-1";
        let mut out_ptr: *const u8 = std::ptr::null();
        let mut out_len: u32 = 0;
        let result = unsafe {
            reader_list_bookmarks(
                book_id.as_ptr(),
                book_id.len() as u32,
                &mut out_ptr,
                &mut out_len,
            )
        };
        assert_eq!(result, FFI_OK);
        let json_str =
            unsafe { std::str::from_utf8(slice::from_raw_parts(out_ptr, out_len as usize)) }
                .unwrap();
        assert!(json_str.contains("bm-1"));
        assert!(json_str.contains("Chapter 1"));

        // Delete bookmark
        let bm_id = b"bm-1";
        let result = unsafe {
            reader_delete_bookmark(bm_id.as_ptr(), bm_id.len() as u32, 2000)
        };
        assert_eq!(result, FFI_OK);

        // Verify empty after delete
        let result = unsafe {
            reader_list_bookmarks(
                book_id.as_ptr(),
                book_id.len() as u32,
                &mut out_ptr,
                &mut out_len,
            )
        };
        assert_eq!(result, FFI_OK);
        let json_str =
            unsafe { std::str::from_utf8(slice::from_raw_parts(out_ptr, out_len as usize)) }
                .unwrap();
        assert_eq!(json_str, "[]");

        unsafe { reader_engine_destroy() };
    }

    #[test]
    fn test_delete_nonexistent_bookmark() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);

        let bm_id = b"nonexistent-bm";
        let result = unsafe {
            reader_delete_bookmark(bm_id.as_ptr(), bm_id.len() as u32, 1000)
        };
        assert_eq!(result, FFI_ERR_NOT_FOUND);

        unsafe { reader_engine_destroy() };
    }

    #[test]
    fn test_list_bookmarks_empty() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);

        let book_id = b"book-1";
        let mut out_ptr: *const u8 = std::ptr::null();
        let mut out_len: u32 = 0;
        let result = unsafe {
            reader_list_bookmarks(
                book_id.as_ptr(),
                book_id.len() as u32,
                &mut out_ptr,
                &mut out_len,
            )
        };
        assert_eq!(result, FFI_OK);
        let json_str =
            unsafe { std::str::from_utf8(slice::from_raw_parts(out_ptr, out_len as usize)) }
                .unwrap();
        assert_eq!(json_str, "[]");

        unsafe { reader_engine_destroy() };
    }

    // ─── Annotation Operations Tests ─────────────────────────────────────────

    #[test]
    fn test_annotation_crud_via_ffi() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);
        register_test_book("book-1");
        // Add annotation (use r##""## to allow # inside the string)
        let ann_json = r##"{"id":"ann-1","book_id":"book-1","cfi_start":"/6/4!/4/2:0","cfi_end":"/6/4!/4/2:10","color_rgba":"#FF0000FF","note":"Important","created_at":1000}"##;
        let result = unsafe {
            reader_add_annotation(ann_json.as_ptr(), ann_json.len() as u32)
        };
        assert_eq!(result, FFI_OK);

        // List annotations
        let book_id = b"book-1";
        let mut out_ptr: *const u8 = std::ptr::null();
        let mut out_len: u32 = 0;
        let result = unsafe {
            reader_list_annotations(
                book_id.as_ptr(),
                book_id.len() as u32,
                &mut out_ptr,
                &mut out_len,
            )
        };
        assert_eq!(result, FFI_OK);
        let json_str =
            unsafe { std::str::from_utf8(slice::from_raw_parts(out_ptr, out_len as usize)) }
                .unwrap();
        assert!(json_str.contains("ann-1"));
        assert!(json_str.contains("Important"));

        // Delete annotation
        let ann_id = b"ann-1";
        let result = unsafe {
            reader_delete_annotation(ann_id.as_ptr(), ann_id.len() as u32, 2000)
        };
        assert_eq!(result, FFI_OK);

        // Verify empty after delete
        let result = unsafe {
            reader_list_annotations(
                book_id.as_ptr(),
                book_id.len() as u32,
                &mut out_ptr,
                &mut out_len,
            )
        };
        assert_eq!(result, FFI_OK);
        let json_str =
            unsafe { std::str::from_utf8(slice::from_raw_parts(out_ptr, out_len as usize)) }
                .unwrap();
        assert_eq!(json_str, "[]");

        unsafe { reader_engine_destroy() };
    }

    #[test]
    fn test_delete_nonexistent_annotation() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);

        let ann_id = b"nonexistent-ann";
        let result = unsafe {
            reader_delete_annotation(ann_id.as_ptr(), ann_id.len() as u32, 1000)
        };
        assert_eq!(result, FFI_ERR_NOT_FOUND);

        unsafe { reader_engine_destroy() };
    }

    #[test]
    fn test_list_annotations_empty() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);

        let book_id = b"book-1";
        let mut out_ptr: *const u8 = std::ptr::null();
        let mut out_len: u32 = 0;
        let result = unsafe {
            reader_list_annotations(
                book_id.as_ptr(),
                book_id.len() as u32,
                &mut out_ptr,
                &mut out_len,
            )
        };
        assert_eq!(result, FFI_OK);
        let json_str =
            unsafe { std::str::from_utf8(slice::from_raw_parts(out_ptr, out_len as usize)) }
                .unwrap();
        assert_eq!(json_str, "[]");

        unsafe { reader_engine_destroy() };
    }

    // ─── Invalid JSON Input Tests ────────────────────────────────────────────

    #[test]
    fn test_add_bookmark_invalid_json() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);

        let bad_json = b"not valid json";
        let result = unsafe {
            reader_add_bookmark(bad_json.as_ptr(), bad_json.len() as u32)
        };
        assert_eq!(result, FFI_ERR_INVALID_UTF8);

        unsafe { reader_engine_destroy() };
    }

    #[test]
    fn test_add_annotation_invalid_json() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);

        let bad_json = b"not valid json";
        let result = unsafe {
            reader_add_annotation(bad_json.as_ptr(), bad_json.len() as u32)
        };
        assert_eq!(result, FFI_ERR_INVALID_UTF8);

        unsafe { reader_engine_destroy() };
    }

    // ─── Close Book Tests ────────────────────────────────────────────────────

    #[test]
    fn test_close_book_without_open() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);

        // Close without opening should succeed (sets current_book to None)
        let result = unsafe { reader_close_book() };
        assert_eq!(result, FFI_OK);

        unsafe { reader_engine_destroy() };
    }

    // ─── Null Pointer for List/Get Operations ────────────────────────────────

    #[test]
    fn test_list_bookmarks_null_out_ptrs() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);

        let book_id = b"book-1";
        let result = unsafe {
            reader_list_bookmarks(
                book_id.as_ptr(),
                book_id.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(result, FFI_ERR_NULL_PTR);

        unsafe { reader_engine_destroy() };
    }

    #[test]
    fn test_list_annotations_null_out_ptrs() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);

        let book_id = b"book-1";
        let result = unsafe {
            reader_list_annotations(
                book_id.as_ptr(),
                book_id.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(result, FFI_ERR_NULL_PTR);

        unsafe { reader_engine_destroy() };
    }

    // ─── resolve_toc_href FFI Tests ──────────────────────────────────────────

    #[test]
    fn test_resolve_toc_href_no_book() {
        let _lock = acquire_lock();
        reset_engine();
        let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);

        let href = b"chapter01.xhtml";
        let result = unsafe {
            reader_resolve_toc_href(href.as_ptr(), href.len() as u32)
        };
        assert_eq!(result, FFI_ERR_NOT_FOUND);

        unsafe { reader_engine_destroy() };
    }

    #[test]
    fn test_resolve_toc_href_null_ptr() {
        let _lock = acquire_lock();
        reset_engine();
        let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);

        let result = unsafe {
            reader_resolve_toc_href(std::ptr::null(), 0)
        };
        assert_eq!(result, FFI_ERR_NULL_PTR);

        unsafe { reader_engine_destroy() };
    }

    #[test]
    fn test_resolve_toc_href_not_init() {
        let _lock = acquire_lock();
        reset_engine();

        let href = b"chapter01.xhtml";
        let result = unsafe {
            reader_resolve_toc_href(href.as_ptr(), href.len() as u32)
        };
        assert_eq!(result, FFI_ERR_NOT_INIT);
    }

    #[test]
    fn test_resolve_toc_href_invalid_utf8() {
        let _lock = acquire_lock();
        reset_engine();
        let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);

        let invalid_bytes: &[u8] = &[0xFF, 0xFE, 0xFD];
        let result = unsafe {
            reader_resolve_toc_href(invalid_bytes.as_ptr(), invalid_bytes.len() as u32)
        };
        assert_eq!(result, FFI_ERR_INVALID_UTF8);

        unsafe { reader_engine_destroy() };
    }

    #[test]
    fn test_get_progress_null_out_ptrs() {
        let _lock = acquire_lock();
        reset_engine();
let (_dir, db_path) = temp_db_path();
        init_engine_with_path(&db_path);

        let book_id = b"book-1";
        let result = unsafe {
            reader_get_progress(
                book_id.as_ptr(),
                book_id.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(result, FFI_ERR_NULL_PTR);

        unsafe { reader_engine_destroy() };
    }
}
