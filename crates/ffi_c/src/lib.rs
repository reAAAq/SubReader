//! C-ABI bridge layer for native platforms (Flutter, iOS, Android, etc.).
//!
//! All exported functions use fixed-width scalar types only (i32, u32, u64, *const u8).
//! Strings are passed as pointer-length pairs (*const u8 + u32).
//! All functions use `catch_unwind` to prevent panics from crossing the FFI boundary.

use std::panic::catch_unwind;
use std::slice;
use std::sync::{Arc, Mutex};

use core_parser::EpubParser;
use core_parser::TxtParser;
use core_state::StateManager;
use shared_types::{Annotation, Bookmark};

use core_auth::http_auth::HttpAuthProvider;
use core_auth::token_store::FileTokenStore;
use core_auth::AuthManager;

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
pub const FFI_ERR_INVALID_JSON: i32 = -100;

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

/// Initialize the reader engine with a database path and optional backend URL.
///
/// If `base_url_ptr` is non-null, an `AuthManager` will be created for authentication.
/// The token file is stored alongside the database as `<db_dir>/auth_token.json`.
///
/// # Safety
/// `db_path_ptr` must point to valid UTF-8 bytes of length `db_path_len`.
/// `base_url_ptr` may be null (auth disabled) or must point to valid UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn reader_engine_init(
    db_path_ptr: *const u8,
    db_path_len: u32,
    device_id_ptr: *const u8,
    device_id_len: u32,
    base_url_ptr: *const u8,
    base_url_len: u32,
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

                // Reset auth/sync globals for this engine instance.
                if let Ok(mut guard) = SYNC_SCHEDULER.lock() {
                    *guard = None;
                } else {
                    return FFI_ERR_UNKNOWN;
                }

                if let Ok(mut guard) = AUTH_MANAGER.lock() {
                    *guard = None;
                } else {
                    return FFI_ERR_UNKNOWN;
                }

                if let Ok(mut guard) = BASE_URL.lock() {
                    *guard = None;
                } else {
                    return FFI_ERR_UNKNOWN;
                }

                // Initialize AuthManager if base_url is provided
                if !base_url_ptr.is_null() && base_url_len > 0 {
                    let base_url = match unsafe { ptr_to_str(base_url_ptr, base_url_len) } {
                        Ok(s) => s,
                        Err(code) => return code,
                    };

                    let provider = HttpAuthProvider::new(base_url);

                    if let Ok(mut guard) = BASE_URL.lock() {
                        *guard = Some(base_url.to_string());
                    } else {
                        return FFI_ERR_UNKNOWN;
                    }

                    // Store token file next to the database
                    let db_dir = std::path::Path::new(db_path)
                        .parent()
                        .unwrap_or(std::path::Path::new("."));
                    let token_path = db_dir.join("auth_token.json");
                    let store = FileTokenStore::new(token_path);

                    let auth_mgr = Arc::new(AuthManager::new(
                        provider,
                        store,
                        device_id.to_string(),
                    ));

                    if let Ok(mut guard) = AUTH_MANAGER.lock() {
                        *guard = Some(Arc::clone(&auth_mgr));
                    } else {
                        return FFI_ERR_UNKNOWN;
                    }

                    // Fire initial auth state callback
                    let rt = match get_runtime() {
                        Ok(rt) => rt,
                        Err(_) => return FFI_OK, // Engine init succeeded, auth callback is best-effort
                    };
                    let state = rt.block_on(auth_mgr.state());
                    fire_auth_callback(&state);

                    // If we have a persisted token, try background refresh
                    if matches!(state, core_auth::AuthState::NeedsRefresh) {
                        let rt_handle = rt.handle().clone();
                        std::thread::spawn(move || {
                            let result = rt_handle.block_on(auth_mgr.refresh());
                            let new_state = rt_handle.block_on(auth_mgr.state());
                            fire_auth_callback(&new_state);
                            if let Err(e) = result {
                                eprintln!("[ffi_c] Background token refresh failed: {}", e);
                            }
                        });
                    }
                }

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
        let rt = match get_runtime() {
            Ok(rt) => Some(rt),
            Err(_) => None,
        };

        {
            let mut guard = match SYNC_SCHEDULER.lock() {
                Ok(g) => g,
                Err(_) => return FFI_ERR_UNKNOWN,
            };

            if let Some(scheduler) = guard.as_mut() {
                if let Some(rt) = rt {
                    rt.block_on(scheduler.stop());
                }
            }
            *guard = None;
        }

        {
            let mut guard = match AUTH_MANAGER.lock() {
                Ok(g) => g,
                Err(_) => return FFI_ERR_UNKNOWN,
            };
            *guard = None;
        }

        {
            let mut guard = match BASE_URL.lock() {
                Ok(g) => g,
                Err(_) => return FFI_ERR_UNKNOWN,
            };
            *guard = None;
        }

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
            Err(_) => return FFI_ERR_INVALID_JSON,
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
            Err(_) => return FFI_ERR_INVALID_JSON,
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

// ─── Auth & Sync Error Codes ─────────────────────────────────────────────────

pub const FFI_ERR_AUTH: i32 = -10;
pub const FFI_ERR_NETWORK: i32 = -11;
pub const FFI_ERR_SYNC: i32 = -12;

// ─── Auth & Sync Global State ────────────────────────────────────────────────

/// Callback function pointer type for auth state changes.
/// The i32 parameter is the auth state: 0=LoggedOut, 1=Authenticated, 2=NeedsRefresh, 3=NeedsReLogin.
pub type AuthCallbackFn = Option<extern "C" fn(i32)>;

/// Callback function pointer type for sync state changes.
/// The i32 parameter is the sync state: 0=Idle, 1=Syncing, 2=Error, 3=Offline, 4=Dormant.
pub type SyncCallbackFn = Option<extern "C" fn(i32)>;

static AUTH_CALLBACK: Mutex<AuthCallbackFn> = Mutex::new(None);
static SYNC_CALLBACK: Mutex<SyncCallbackFn> = Mutex::new(None);

/// Global AuthManager instance, initialized during `reader_engine_init`.
/// Uses concrete types: HttpAuthProvider for network calls, FileTokenStore for persistence.
static AUTH_MANAGER: Mutex<Option<Arc<AuthManager<HttpAuthProvider, FileTokenStore>>>> =
    Mutex::new(None);

/// Tokio runtime for async operations.
/// Uses `OnceLock` to avoid Mutex ordering issues with ENGINE.
static TOKIO_RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();

/// Global base_url storage, set during `reader_engine_init`.
static BASE_URL: Mutex<Option<String>> = Mutex::new(None);

/// Ensure the tokio runtime is initialized and return a reference to it.
fn get_runtime() -> Result<&'static tokio::runtime::Runtime, i32> {
    if let Some(rt) = TOKIO_RT.get() {
        return Ok(rt);
    }
    // First call: create the runtime and store it.
    // If two threads race here, only one will win the set(); the other's
    // Runtime is dropped harmlessly and we return the winner.
    match tokio::runtime::Runtime::new() {
        Ok(rt) => Ok(TOKIO_RT.get_or_init(|| rt)),
        Err(_) => Err(FFI_ERR_UNKNOWN),
    }
}

/// Get a cloned handle to the global AuthManager, or return FFI_ERR_NOT_INIT.
fn get_auth_manager() -> Result<Arc<AuthManager<HttpAuthProvider, FileTokenStore>>, i32> {
    let guard = AUTH_MANAGER.lock().map_err(|_| FFI_ERR_UNKNOWN)?;
    guard.clone().ok_or(FFI_ERR_NOT_INIT)
}

fn current_base_url() -> Result<String, i32> {
    let guard = BASE_URL.lock().map_err(|_| FFI_ERR_UNKNOWN)?;
    guard.clone().ok_or(FFI_ERR_NOT_INIT)
}

type FfiSyncEngine = SyncEngine<StorageAdapter, NetworkSyncAdapter<HttpTransport>>;

fn build_sync_engine() -> Result<FfiSyncEngine, i32> {
    let base_url = current_base_url()?;
    let transport = HttpTransport::new(&base_url);
    let adapter = NetworkSyncAdapter::new(transport);
    let storage = StorageAdapter;

    // Use a simple node_id; AuthManager doesn't expose device_id directly.
    let node_id: u32 = 1;

    Ok(SyncEngine::new(storage, adapter, "ffi".to_string(), node_id))
}

/// Convert an AuthState to its FFI i32 representation.
fn auth_state_to_i32(state: &core_auth::AuthState) -> i32 {
    match state {
        core_auth::AuthState::LoggedOut => 0,
        core_auth::AuthState::Authenticated { .. } => 1,
        core_auth::AuthState::NeedsRefresh => 2,
        core_auth::AuthState::NeedsReLogin => 3,
    }
}

/// Fire the auth callback with the current auth state.
fn fire_auth_callback(state: &core_auth::AuthState) {
    let code = auth_state_to_i32(state);
    if let Ok(guard) = AUTH_CALLBACK.lock() {
        if let Some(cb) = *guard {
            cb(code);
        }
    }
}

/// Map an AuthError to an FFI error code.
fn auth_error_to_ffi(err: &core_auth::AuthError) -> i32 {
    match err {
        core_auth::AuthError::NetworkError(_) => FFI_ERR_NETWORK,
        _ => FFI_ERR_AUTH,
    }
}

// ─── Auth FFI Functions ──────────────────────────────────────────────────────

/// Register a new user account.
/// Uses the global AuthManager initialized during `reader_engine_init`.
///
/// # Safety
/// All pointer parameters must be valid UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn reader_auth_register(
    username_ptr: *const u8,
    username_len: u32,
    email_ptr: *const u8,
    email_len: u32,
    password_ptr: *const u8,
    password_len: u32,
    out_ptr: *mut *const u8,
    out_len: *mut u32,
) -> i32 {
    catch_unwind(|| {
        let username = match unsafe { ptr_to_str(username_ptr, username_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };
        let email = match unsafe { ptr_to_str(email_ptr, email_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };
        let password = match unsafe { ptr_to_str(password_ptr, password_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };

        if out_ptr.is_null() || out_len.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let rt = match get_runtime() {
            Ok(rt) => rt,
            Err(code) => return code,
        };

        let mgr = match get_auth_manager() {
            Ok(m) => m,
            Err(code) => return code,
        };

        let req = core_auth::RegisterRequest {
            username: username.to_string(),
            email: email.to_string(),
            password: password.to_string(),
        };

        match rt.block_on(mgr.register(&req)) {
            Ok(user_id) => {
                let mut guard = match ENGINE.lock() {
                    Ok(g) => g,
                    Err(_) => return FFI_ERR_UNKNOWN,
                };
                let engine = match guard.as_mut() {
                    Some(e) => e,
                    None => return FFI_ERR_NOT_INIT,
                };
                let (ptr, len) = set_return_buffer(engine, user_id);
                unsafe {
                    *out_ptr = ptr;
                    *out_len = len;
                }
                FFI_OK
            }
            Err(e) => auth_error_to_ffi(&e),
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

fn auth_login_impl(
    credential_ptr: *const u8,
    credential_len: u32,
    password_ptr: *const u8,
    password_len: u32,
    device_name_ptr: *const u8,
    device_name_len: u32,
    platform_ptr: *const u8,
    platform_len: u32,
    out_ptr: *mut *const u8,
    out_len: *mut u32,
) -> i32 {
    catch_unwind(|| {
        let credential = match unsafe { ptr_to_str(credential_ptr, credential_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };
        let password = match unsafe { ptr_to_str(password_ptr, password_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };
        let device_name = if device_name_ptr.is_null() {
            None
        } else {
            match unsafe { ptr_to_str(device_name_ptr, device_name_len) } {
                Ok(s) => Some(s),
                Err(code) => return code,
            }
        };
        let platform = if platform_ptr.is_null() {
            None
        } else {
            match unsafe { ptr_to_str(platform_ptr, platform_len) } {
                Ok(s) => Some(s),
                Err(code) => return code,
            }
        };

        if out_ptr.is_null() || out_len.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let rt = match get_runtime() {
            Ok(rt) => rt,
            Err(code) => return code,
        };

        let mgr = match get_auth_manager() {
            Ok(m) => m,
            Err(code) => return code,
        };

        match rt.block_on(mgr.login(credential, password, device_name, platform)) {
            Ok(token) => {
                // Notify auth state change
                let state = rt.block_on(mgr.state());
                fire_auth_callback(&state);

                let json = match serde_json::to_string(&token) {
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
            }
            Err(e) => auth_error_to_ffi(&e),
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Login with credentials. Returns JSON with token info.
/// Uses the global AuthManager; device_id is already stored in AuthManager.
///
/// # Safety
/// All pointer parameters must be valid.
#[no_mangle]
pub unsafe extern "C" fn reader_auth_login(
    credential_ptr: *const u8,
    credential_len: u32,
    password_ptr: *const u8,
    password_len: u32,
    out_ptr: *mut *const u8,
    out_len: *mut u32,
) -> i32 {
    auth_login_impl(
        credential_ptr,
        credential_len,
        password_ptr,
        password_len,
        std::ptr::null(),
        0,
        std::ptr::null(),
        0,
        out_ptr,
        out_len,
    )
}

/// Login with credentials and optional device metadata. Returns JSON with token info.
/// Uses the global AuthManager; device_id is already stored in AuthManager.
///
/// # Safety
/// All non-null pointer parameters must be valid.
#[no_mangle]
pub unsafe extern "C" fn reader_auth_login_with_metadata(
    credential_ptr: *const u8,
    credential_len: u32,
    password_ptr: *const u8,
    password_len: u32,
    device_name_ptr: *const u8,
    device_name_len: u32,
    platform_ptr: *const u8,
    platform_len: u32,
    out_ptr: *mut *const u8,
    out_len: *mut u32,
) -> i32 {
    auth_login_impl(
        credential_ptr,
        credential_len,
        password_ptr,
        password_len,
        device_name_ptr,
        device_name_len,
        platform_ptr,
        platform_len,
        out_ptr,
        out_len,
    )
}

/// Logout the current session.
/// Uses the global AuthManager to clear tokens and update state.
///
/// # Safety
/// No pointer parameters required.
#[no_mangle]
pub unsafe extern "C" fn reader_auth_logout() -> i32 {
    catch_unwind(|| {
        let rt = match get_runtime() {
            Ok(rt) => rt,
            Err(code) => return code,
        };

        let mgr = match get_auth_manager() {
            Ok(m) => m,
            Err(code) => return code,
        };

        match rt.block_on(mgr.logout()) {
            Ok(()) => {
                let state = rt.block_on(mgr.state());
                fire_auth_callback(&state);
                FFI_OK
            }
            Err(e) => auth_error_to_ffi(&e),
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Get the current authentication state.
///
/// Returns: 0=LoggedOut, 1=Authenticated, 2=NeedsRefresh, 3=NeedsReLogin.
/// Returns FFI_ERR_NOT_INIT if AuthManager is not initialized.
#[no_mangle]
pub unsafe extern "C" fn reader_auth_get_state() -> i32 {
    catch_unwind(|| {
        let rt = match get_runtime() {
            Ok(rt) => rt,
            Err(code) => return code,
        };

        let mgr = match get_auth_manager() {
            Ok(m) => m,
            Err(code) => return code,
        };

        let state = rt.block_on(mgr.state());
        auth_state_to_i32(&state)
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Get a valid access token, auto-refreshing if needed.
/// Returns the token string via out_ptr/out_len.
///
/// # Safety
/// `out_ptr` and `out_len` must be valid pointers.
#[no_mangle]
pub unsafe extern "C" fn reader_auth_get_token(
    out_ptr: *mut *const u8,
    out_len: *mut u32,
) -> i32 {
    catch_unwind(|| {
        if out_ptr.is_null() || out_len.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let rt = match get_runtime() {
            Ok(rt) => rt,
            Err(code) => return code,
        };

        let mgr = match get_auth_manager() {
            Ok(m) => m,
            Err(code) => return code,
        };

        match rt.block_on(mgr.get_valid_token()) {
            Ok(token) => {
                // Fire callback in case state changed during refresh
                let state = rt.block_on(mgr.state());
                fire_auth_callback(&state);

                let mut guard = match ENGINE.lock() {
                    Ok(g) => g,
                    Err(_) => return FFI_ERR_UNKNOWN,
                };
                let engine = match guard.as_mut() {
                    Some(e) => e,
                    None => return FFI_ERR_NOT_INIT,
                };
                let (ptr, len) = set_return_buffer(engine, token);
                unsafe {
                    *out_ptr = ptr;
                    *out_len = len;
                }
                FFI_OK
            }
            Err(e) => {
                let state = rt.block_on(mgr.state());
                fire_auth_callback(&state);
                auth_error_to_ffi(&e)
            }
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Explicitly refresh the access token.
/// Returns the new token JSON via out_ptr/out_len.
///
/// # Safety
/// `out_ptr` and `out_len` must be valid pointers.
#[no_mangle]
pub unsafe extern "C" fn reader_auth_refresh_token(
    out_ptr: *mut *const u8,
    out_len: *mut u32,
) -> i32 {
    catch_unwind(|| {
        if out_ptr.is_null() || out_len.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let rt = match get_runtime() {
            Ok(rt) => rt,
            Err(code) => return code,
        };

        let mgr = match get_auth_manager() {
            Ok(m) => m,
            Err(code) => return code,
        };

        match rt.block_on(mgr.refresh()) {
            Ok(token) => {
                let state = rt.block_on(mgr.state());
                fire_auth_callback(&state);

                let json = match serde_json::to_string(&token) {
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
            }
            Err(e) => {
                let state = rt.block_on(mgr.state());
                fire_auth_callback(&state);
                auth_error_to_ffi(&e)
            }
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Change the current user's password.
///
/// # Safety
/// All pointer parameters must be valid UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn reader_auth_change_password(
    old_password_ptr: *const u8,
    old_password_len: u32,
    new_password_ptr: *const u8,
    new_password_len: u32,
) -> i32 {
    catch_unwind(|| {
        let old_password = match unsafe { ptr_to_str(old_password_ptr, old_password_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };
        let new_password = match unsafe { ptr_to_str(new_password_ptr, new_password_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };

        let rt = match get_runtime() {
            Ok(rt) => rt,
            Err(code) => return code,
        };

        let mgr = match get_auth_manager() {
            Ok(m) => m,
            Err(code) => return code,
        };

        // Get a valid token first (auto-refresh if needed)
        let access_token = match rt.block_on(mgr.get_valid_token()) {
            Ok(t) => t,
            Err(e) => {
                let state = rt.block_on(mgr.state());
                fire_auth_callback(&state);
                return auth_error_to_ffi(&e);
            }
        };

        // AuthManager now exposes change_password, list_devices, remove_device
        // which delegate to the underlying provider.
        match rt.block_on(mgr.change_password(&access_token, old_password, new_password)) {
            Ok(()) => FFI_OK,
            Err(e) => auth_error_to_ffi(&e),
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// List devices for the current user.
/// Returns JSON array via out_ptr/out_len.
///
/// # Safety
/// `out_ptr` and `out_len` must be valid pointers.
#[no_mangle]
pub unsafe extern "C" fn reader_auth_list_devices(
    out_ptr: *mut *const u8,
    out_len: *mut u32,
) -> i32 {
    catch_unwind(|| {
        if out_ptr.is_null() || out_len.is_null() {
            return FFI_ERR_NULL_PTR;
        }

        let rt = match get_runtime() {
            Ok(rt) => rt,
            Err(code) => return code,
        };

        let mgr = match get_auth_manager() {
            Ok(m) => m,
            Err(code) => return code,
        };

        let access_token = match rt.block_on(mgr.get_valid_token()) {
            Ok(t) => t,
            Err(e) => {
                let state = rt.block_on(mgr.state());
                fire_auth_callback(&state);
                return auth_error_to_ffi(&e);
            }
        };

        match rt.block_on(mgr.list_devices(&access_token)) {
            Ok(devices) => {
                let json = match serde_json::to_string(&devices) {
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
            }
            Err(e) => auth_error_to_ffi(&e),
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Remove a device from the current user's device list.
///
/// # Safety
/// `device_id_ptr` must be valid UTF-8.
#[no_mangle]
pub unsafe extern "C" fn reader_auth_remove_device(
    device_id_ptr: *const u8,
    device_id_len: u32,
) -> i32 {
    catch_unwind(|| {
        let device_id = match unsafe { ptr_to_str(device_id_ptr, device_id_len) } {
            Ok(s) => s,
            Err(code) => return code,
        };

        let rt = match get_runtime() {
            Ok(rt) => rt,
            Err(code) => return code,
        };

        let mgr = match get_auth_manager() {
            Ok(m) => m,
            Err(code) => return code,
        };

        let access_token = match rt.block_on(mgr.get_valid_token()) {
            Ok(t) => t,
            Err(e) => {
                let state = rt.block_on(mgr.state());
                fire_auth_callback(&state);
                return auth_error_to_ffi(&e);
            }
        };

        match rt.block_on(mgr.remove_device(&access_token, device_id)) {
            Ok(()) => FFI_OK,
            Err(e) => auth_error_to_ffi(&e),
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Set the auth state change callback.
///
/// # Safety
/// `callback` must be a valid function pointer or null.
#[no_mangle]
pub unsafe extern "C" fn reader_set_auth_callback(callback: AuthCallbackFn) -> i32 {
    catch_unwind(|| {
        let mut guard = match AUTH_CALLBACK.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };
        *guard = callback;
        FFI_OK
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Set the sync state change callback.
///
/// # Safety
/// `callback` must be a valid function pointer or null.
#[no_mangle]
pub unsafe extern "C" fn reader_set_sync_callback(callback: SyncCallbackFn) -> i32 {
    catch_unwind(|| {
        let mut guard = match SYNC_CALLBACK.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };
        *guard = callback;
        FFI_OK
    })
    .unwrap_or(FFI_ERR_PANIC)
}

// ─── Sync Infrastructure ─────────────────────────────────────────────────────

use core_sync::engine::{SyncEngine, SyncStorage};
use core_sync::scheduler::{SyncScheduler, SyncState, TokenProvider};
use core_network::http_transport::HttpTransport;
use core_network::NetworkSyncAdapter;

/// Adapter that bridges `core_storage::Database` (via `EngineState`) to the
/// `SyncStorage` trait required by `SyncEngine`.
///
/// Because `Database` uses `rusqlite::Connection` (which contains `RefCell`
/// and is not `Sync`), we cannot hold a direct `&Database` across threads.
/// Instead, each method acquires the global `ENGINE` mutex, which is the
/// same pattern used by all other FFI functions.
struct StorageAdapter;

impl SyncStorage for StorageAdapter {
    fn get_unsynced_ops(
        &self,
    ) -> Result<Vec<core_sync::engine::UnsyncedOp>, core_sync::SyncError> {
        let mut guard = ENGINE
            .lock()
            .map_err(|_| core_sync::SyncError::Storage("Engine lock poisoned".into()))?;
        let engine = guard
            .as_mut()
            .ok_or_else(|| core_sync::SyncError::Storage("Engine not initialized".into()))?;

        engine
            .state_manager
            .database()
            .get_unsynced_ops()
            .map(|ops| {
                ops.into_iter()
                    .map(|(id, op_type, op_data, hlc_ts, device_id)| {
                        core_sync::engine::UnsyncedOp {
                            local_id: id,
                            op_id: uuid::Uuid::new_v4().to_string(),
                            op_type,
                            op_data,
                            hlc_ts,
                            device_id,
                        }
                    })
                    .collect()
            })
            .map_err(|e| core_sync::SyncError::Storage(e.to_string()))
    }

    fn mark_ops_synced(&self, local_ids: &[i64]) -> Result<(), core_sync::SyncError> {
        let mut guard = ENGINE
            .lock()
            .map_err(|_| core_sync::SyncError::Storage("Engine lock poisoned".into()))?;
        let engine = guard
            .as_mut()
            .ok_or_else(|| core_sync::SyncError::Storage("Engine not initialized".into()))?;

        engine
            .state_manager
            .database()
            .mark_ops_synced(local_ids)
            .map_err(|e| core_sync::SyncError::Storage(e.to_string()))
    }

    fn get_sync_meta(&self, key: &str) -> Result<Option<String>, core_sync::SyncError> {
        let mut guard = ENGINE
            .lock()
            .map_err(|_| core_sync::SyncError::Storage("Engine lock poisoned".into()))?;
        let engine = guard
            .as_mut()
            .ok_or_else(|| core_sync::SyncError::Storage("Engine not initialized".into()))?;

        engine
            .state_manager
            .database()
            .get_sync_meta(key)
            .map_err(|e| core_sync::SyncError::Storage(e.to_string()))
    }

    fn set_sync_meta(&self, key: &str, value: &str) -> Result<(), core_sync::SyncError> {
        let mut guard = ENGINE
            .lock()
            .map_err(|_| core_sync::SyncError::Storage("Engine lock poisoned".into()))?;
        let engine = guard
            .as_mut()
            .ok_or_else(|| core_sync::SyncError::Storage("Engine not initialized".into()))?;

        engine
            .state_manager
            .database()
            .set_sync_meta(key, value)
            .map_err(|e| core_sync::SyncError::Storage(e.to_string()))
    }

    fn apply_remote_op(
        &self,
        op: &core_sync::engine::RemoteOp,
    ) -> Result<bool, core_sync::SyncError> {
        let mut guard = ENGINE
            .lock()
            .map_err(|_| core_sync::SyncError::Storage("Engine lock poisoned".into()))?;
        let engine = guard
            .as_mut()
            .ok_or_else(|| core_sync::SyncError::Storage("Engine not initialized".into()))?;

        engine
            .state_manager
            .database()
            .apply_remote_op(&op.op_type, &op.op_data, op.hlc_ts, &op.device_id)
            .map_err(|e| core_sync::SyncError::Storage(e.to_string()))
    }
}

/// Token provider that delegates to the global AuthManager.
struct AuthManagerTokenProvider;

impl TokenProvider for AuthManagerTokenProvider {
    fn get_token(&self) -> Option<String> {
        let mgr = get_auth_manager().ok()?;
        let rt = get_runtime().ok()?;
        rt.block_on(mgr.get_valid_token()).ok()
    }
}

/// Type alias for the concrete SyncScheduler used in FFI.
type FfiSyncScheduler = SyncScheduler<StorageAdapter, NetworkSyncAdapter<HttpTransport>>;

/// Global SyncScheduler instance.
static SYNC_SCHEDULER: Mutex<Option<FfiSyncScheduler>> = Mutex::new(None);

/// Convert a SyncState to its FFI i32 representation.
fn sync_state_to_i32(state: &SyncState) -> i32 {
    match state {
        SyncState::Idle => 0,
        SyncState::Syncing => 1,
        SyncState::Error => 2,
        SyncState::Offline => 3,
        SyncState::Dormant => 4,
    }
}

/// Fire the sync callback with the given state.
fn fire_sync_callback(state: &SyncState) {
    let code = sync_state_to_i32(state);
    if let Ok(guard) = SYNC_CALLBACK.lock() {
        if let Some(cb) = *guard {
            cb(code);
        }
    }
}

/// Map a SyncError to an FFI error code.
fn sync_error_to_ffi(err: &core_sync::SyncError) -> i32 {
    match err {
        core_sync::SyncError::NotAuthenticated => FFI_ERR_AUTH,
        core_sync::SyncError::Transport(_) => FFI_ERR_NETWORK,
        _ => FFI_ERR_SYNC,
    }
}

/// Helper: ensure the SyncScheduler is created (but not necessarily started).
/// Requires that AUTH_MANAGER is already initialized (i.e., base_url was provided).
fn ensure_sync_scheduler() -> Result<(), i32> {
    let mut guard = SYNC_SCHEDULER.lock().map_err(|_| FFI_ERR_UNKNOWN)?;
    if guard.is_some() {
        return Ok(());
    }

    let engine = build_sync_engine()?;
    let scheduler = SyncScheduler::new(engine);
    *guard = Some(scheduler);
    Ok(())
}

fn with_valid_sync_token(
    rt: &'static tokio::runtime::Runtime,
) -> Result<(Arc<AuthManager<HttpAuthProvider, FileTokenStore>>, String), i32> {
    let mgr = get_auth_manager().map_err(|_| FFI_ERR_AUTH)?;
    match rt.block_on(mgr.get_valid_token()) {
        Ok(token) => Ok((mgr, token)),
        Err(_) => Err(FFI_ERR_AUTH),
    }
}

fn run_manual_sync<F>(operation: F) -> i32
where
    F: FnOnce(&'static tokio::runtime::Runtime, &FfiSyncEngine, &str) -> Result<(), core_sync::SyncError>,
{
    let rt = match get_runtime() {
        Ok(rt) => rt,
        Err(code) => return code,
    };

    let (mgr, token) = match with_valid_sync_token(rt) {
        Ok(value) => value,
        Err(code) => return code,
    };

    let engine = match build_sync_engine() {
        Ok(engine) => engine,
        Err(code) => return code,
    };

    fire_sync_callback(&SyncState::Syncing);

    let result = operation(rt, &engine, &token);

    let auth_state = rt.block_on(mgr.state());
    fire_auth_callback(&auth_state);

    match result {
        Ok(()) => {
            fire_sync_callback(&SyncState::Idle);
            FFI_OK
        }
        Err(e) => {
            let sync_state = match &e {
                core_sync::SyncError::NotAuthenticated => SyncState::Dormant,
                core_sync::SyncError::Transport(_) => SyncState::Offline,
                _ => SyncState::Error,
            };
            fire_sync_callback(&sync_state);
            sync_error_to_ffi(&e)
        }
    }
}

// ─── Sync FFI Functions ──────────────────────────────────────────────────────

/// Push local pending operations to the server.
///
/// # Safety
/// No pointer parameters required.
#[no_mangle]
pub unsafe extern "C" fn reader_sync_push() -> i32 {
    catch_unwind(|| {
        run_manual_sync(|rt, engine, token| {
            rt.block_on(engine.push_pending(token))?;
            Ok(())
        })
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Pull remote operations from the server.
///
/// # Safety
/// No pointer parameters required.
#[no_mangle]
pub unsafe extern "C" fn reader_sync_pull() -> i32 {
    catch_unwind(|| {
        run_manual_sync(|rt, engine, token| {
            rt.block_on(engine.pull_remote(token))?;
            Ok(())
        })
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Perform a full sync (push + pull).
///
/// # Safety
/// No pointer parameters required.
#[no_mangle]
pub unsafe extern "C" fn reader_sync_full() -> i32 {
    catch_unwind(|| {
        run_manual_sync(|rt, engine, token| {
            rt.block_on(engine.sync(token))?;
            Ok(())
        })
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Start the background sync scheduler.
///
/// # Safety
/// No pointer parameters required.
#[no_mangle]
pub unsafe extern "C" fn reader_sync_start_scheduler() -> i32 {
    catch_unwind(|| {
        let rt = match get_runtime() {
            Ok(rt) => rt,
            Err(code) => return code,
        };

        if get_auth_manager().is_err() {
            return FFI_ERR_AUTH;
        }

        if let Err(code) = ensure_sync_scheduler() {
            return code;
        }

        let mut guard = match SYNC_SCHEDULER.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        if let Some(scheduler) = guard.as_mut() {
            rt.block_on(scheduler.set_state_callback(Box::new(|state| {
                fire_sync_callback(&state);
            })));

            let token_provider = Arc::new(AuthManagerTokenProvider);
            scheduler.start(token_provider);
            FFI_OK
        } else {
            FFI_ERR_NOT_INIT
        }
    })
    .unwrap_or(FFI_ERR_PANIC)
}

/// Stop the background sync scheduler.
///
/// # Safety
/// No pointer parameters required.
#[no_mangle]
pub unsafe extern "C" fn reader_sync_stop_scheduler() -> i32 {
    catch_unwind(|| {
        let rt = match get_runtime() {
            Ok(rt) => rt,
            Err(code) => return code,
        };

        let mut guard = match SYNC_SCHEDULER.lock() {
            Ok(g) => g,
            Err(_) => return FFI_ERR_UNKNOWN,
        };

        if let Some(scheduler) = guard.as_mut() {
            rt.block_on(scheduler.stop());
            fire_sync_callback(&SyncState::Dormant);
            FFI_OK
        } else {
            // Not initialized is not an error for stop
            FFI_OK
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
                std::ptr::null(),
                0,
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
        let result = unsafe { reader_engine_init(std::ptr::null(), 0, std::ptr::null(), 0, std::ptr::null(), 0) };
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
        assert_eq!(FFI_ERR_INVALID_JSON, -100);
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
                std::ptr::null(),
                0,
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
                std::ptr::null(),
                0,
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
                std::ptr::null(),
                0,
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
        assert_eq!(result, FFI_ERR_INVALID_JSON);

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
        assert_eq!(result, FFI_ERR_INVALID_JSON);

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
