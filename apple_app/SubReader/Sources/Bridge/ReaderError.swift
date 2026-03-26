// ReaderError — Maps FFI error codes to Swift errors.
//
// Each case corresponds to a constant defined in reader_core.h.

import Foundation
import CReaderCore

/// Error type for all Rust FFI operations.
public enum ReaderError: Int32, Error, Sendable {
    case nullPointer    = -1   // FFI_ERR_NULL_PTR
    case invalidUtf8    = -2   // FFI_ERR_INVALID_UTF8
    case parseFailed    = -3   // FFI_ERR_PARSE_FAILED
    case storage        = -4   // FFI_ERR_STORAGE
    case notFound       = -5   // FFI_ERR_NOT_FOUND
    case alreadyInit    = -6   // FFI_ERR_ALREADY_INIT
    case notInit        = -7   // FFI_ERR_NOT_INIT
    case panic          = -98  // FFI_ERR_PANIC
    case unknown        = -99  // FFI_ERR_UNKNOWN

    /// Create from an FFI return code. Returns nil for FFI_OK (0).
    public static func from(code: Int32) -> ReaderError? {
        if code == FFI_OK { return nil }
        return ReaderError(rawValue: code) ?? .unknown
    }
}

extension ReaderError: LocalizedError {
    public var errorDescription: String? {
        switch self {
        case .nullPointer:  return "Null pointer passed to FFI"
        case .invalidUtf8:  return "Invalid UTF-8 data"
        case .parseFailed:  return "Failed to parse book data"
        case .storage:      return "Database storage error"
        case .notFound:     return "Requested resource not found"
        case .alreadyInit:  return "Engine already initialized"
        case .notInit:      return "Engine not initialized"
        case .panic:        return "Internal engine panic"
        case .unknown:      return "Unknown engine error"
        }
    }
}
