// RustCore — High-performance FFI bridge to the Rust reader engine.
//
// All C-ABI calls execute on a dedicated serial queue (never on main thread).
// Uses zero-copy Data wrapping where possible for JSON responses.

import Foundation
import CReaderCore
import ReaderModels
import os.log

/// Concrete implementation of ReaderEngineProtocol backed by Rust C-ABI.
public final class RustCore: ReaderEngineProtocol, @unchecked Sendable {

    // MARK: - Private Properties

    /// Dedicated serial queue for all FFI calls — keeps main thread free.
    private let ffiQueue = DispatchQueue(label: "com.subreader.ffi", qos: .userInitiated)

    /// Logger for performance tracking and diagnostics.
    private static let logger = Logger(subsystem: "com.subreader.bridge", category: "RustCore")

    /// Whether the engine has been initialized.
    private var isInitialized = false

    // MARK: - Lifecycle

    public init() {}

    deinit {
        if isInitialized {
            let _ = destroy()
        }
    }

    // MARK: - Engine Lifecycle

    public func initialize(dbPath: String, deviceId: String, baseURL: String? = nil) -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let start = CFAbsoluteTimeGetCurrent()
            let code = dbPath.withCString { dbPathCStr in
                deviceId.withCString { deviceIdCStr in
                    if let baseURL = baseURL {
                        return baseURL.withCString { baseURLCStr in
                            reader_engine_init(
                                UnsafeRawPointer(dbPathCStr).assumingMemoryBound(to: UInt8.self),
                                UInt32(dbPath.utf8.count),
                                UnsafeRawPointer(deviceIdCStr).assumingMemoryBound(to: UInt8.self),
                                UInt32(deviceId.utf8.count),
                                UnsafeRawPointer(baseURLCStr).assumingMemoryBound(to: UInt8.self),
                                UInt32(baseURL.utf8.count)
                            )
                        }
                    } else {
                        return reader_engine_init(
                            UnsafeRawPointer(dbPathCStr).assumingMemoryBound(to: UInt8.self),
                            UInt32(dbPath.utf8.count),
                            UnsafeRawPointer(deviceIdCStr).assumingMemoryBound(to: UInt8.self),
                            UInt32(deviceId.utf8.count),
                            nil,
                            0
                        )
                    }
                }
            }
            let elapsed = (CFAbsoluteTimeGetCurrent() - start) * 1000
            Self.logger.debug("reader_engine_init: \(elapsed, format: .fixed(precision: 2))ms")

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            isInitialized = true
            return .success(())
        }
    }

    public func destroy() -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = reader_engine_destroy()
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            isInitialized = false
            return .success(())
        }
    }

    // MARK: - Book Operations

    public func openBook(data: Data) -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let start = CFAbsoluteTimeGetCurrent()
            let code = data.withUnsafeBytes { buffer in
                guard let ptr = buffer.baseAddress?.assumingMemoryBound(to: UInt8.self) else {
                    return FFI_ERR_NULL_PTR
                }
                return reader_open_book(ptr, UInt32(buffer.count))
            }
            let elapsed = (CFAbsoluteTimeGetCurrent() - start) * 1000
            Self.logger.debug("reader_open_book (\(data.count) bytes): \(elapsed, format: .fixed(precision: 2))ms")

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    public func closeBook() -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = reader_close_book()
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    public func getMetadata() -> Result<BookMetadata, ReaderError> {
        ffiQueue.sync {
            let start = CFAbsoluteTimeGetCurrent()
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = reader_get_metadata(&outPtr, &outLen)
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            let result: Result<BookMetadata, ReaderError> = decodeJSON(ptr: outPtr, len: outLen)
            let elapsed = (CFAbsoluteTimeGetCurrent() - start) * 1000
            Self.logger.debug("reader_get_metadata: \(elapsed, format: .fixed(precision: 2))ms")
            return result
        }
    }

    public func getChapterContent(path: String) -> Result<[DomNode], ReaderError> {
        ffiQueue.sync {
            let start = CFAbsoluteTimeGetCurrent()
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = path.withCString { pathCStr in
                reader_get_chapter_content(
                    UnsafeRawPointer(pathCStr).assumingMemoryBound(to: UInt8.self),
                    UInt32(path.utf8.count),
                    &outPtr,
                    &outLen
                )
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            let result: Result<[DomNode], ReaderError> = decodeJSON(ptr: outPtr, len: outLen)
            let elapsed = (CFAbsoluteTimeGetCurrent() - start) * 1000
            Self.logger.debug("reader_get_chapter_content(\(path)): \(elapsed, format: .fixed(precision: 2))ms, \(outLen) bytes")
            return result
        }
    }

    public func getToc() -> Result<[TocEntry], ReaderError> {
        ffiQueue.sync {
            let start = CFAbsoluteTimeGetCurrent()
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = reader_get_toc(&outPtr, &outLen)
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            let result: Result<[TocEntry], ReaderError> = decodeJSON(ptr: outPtr, len: outLen)
            let elapsed = (CFAbsoluteTimeGetCurrent() - start) * 1000
            Self.logger.debug("reader_get_toc: \(elapsed, format: .fixed(precision: 2))ms")
            return result
        }
    }

    public func getSpine() -> Result<[String], ReaderError> {
        ffiQueue.sync {
            let start = CFAbsoluteTimeGetCurrent()
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = reader_get_spine(&outPtr, &outLen)
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            let result: Result<[String], ReaderError> = decodeJSON(ptr: outPtr, len: outLen)
            let elapsed = (CFAbsoluteTimeGetCurrent() - start) * 1000
            Self.logger.debug("reader_get_spine: \(elapsed, format: .fixed(precision: 2))ms")
            return result
        }
    }

    public func getCoverImage(coverId: String) -> Result<Data, ReaderError> {
        ffiQueue.sync {
            let start = CFAbsoluteTimeGetCurrent()
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = coverId.withCString { coverCStr in
                reader_get_cover_image(
                    UnsafeRawPointer(coverCStr).assumingMemoryBound(to: UInt8.self),
                    UInt32(coverId.utf8.count),
                    &outPtr,
                    &outLen
                )
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            guard let ptr = outPtr, outLen > 0 else {
                return .failure(.notFound)
            }

            let data = Data(bytes: ptr, count: Int(outLen))
            let elapsed = (CFAbsoluteTimeGetCurrent() - start) * 1000
            Self.logger.debug("reader_get_cover_image: \(elapsed, format: .fixed(precision: 2))ms, \(outLen) bytes")
            return .success(data)
        }
    }

    public func resolveTocHref(href: String) -> Result<Int, ReaderError> {
        ffiQueue.sync {
            let code = href.withCString { hrefCStr in
                reader_resolve_toc_href(
                    UnsafeRawPointer(hrefCStr).assumingMemoryBound(to: UInt8.self),
                    UInt32(href.utf8.count)
                )
            }

            // Negative error codes (except -1 which means "not matched")
            if code < -1 {
                if let error = ReaderError.from(code: code) {
                    return .failure(error)
                }
                return .failure(.unknown)
            }

            // -1 means no match found
            if code == -1 {
                return .failure(.notFound)
            }

            return .success(Int(code))
        }
    }

    // MARK: - TXT Operations

    public func parseTxt(data: Data) -> Result<TxtParseResult, ReaderError> {
        ffiQueue.sync {
            let start = CFAbsoluteTimeGetCurrent()
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = data.withUnsafeBytes { buffer in
                guard let ptr = buffer.baseAddress?.assumingMemoryBound(to: UInt8.self) else {
                    return FFI_ERR_NULL_PTR
                }
                return reader_parse_txt(ptr, UInt32(buffer.count), &outPtr, &outLen)
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            let result: Result<TxtParseResult, ReaderError> = decodeJSON(ptr: outPtr, len: outLen)
            let elapsed = (CFAbsoluteTimeGetCurrent() - start) * 1000
            Self.logger.debug("reader_parse_txt (\(data.count) bytes): \(elapsed, format: .fixed(precision: 2))ms")
            return result
        }
    }

    // MARK: - Progress

    public func getProgress(bookId: String) -> Result<ReadingProgress, ReaderError> {
        ffiQueue.sync {
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = bookId.withCString { idCStr in
                reader_get_progress(
                    UnsafeRawPointer(idCStr).assumingMemoryBound(to: UInt8.self),
                    UInt32(bookId.utf8.count),
                    &outPtr,
                    &outLen
                )
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            return decodeJSON(ptr: outPtr, len: outLen)
        }
    }

    public func updateProgress(bookId: String, cfi: String, percentage: Double, hlcTs: UInt64) -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = bookId.withCString { idCStr in
                cfi.withCString { cfiCStr in
                    reader_update_progress(
                        UnsafeRawPointer(idCStr).assumingMemoryBound(to: UInt8.self),
                        UInt32(bookId.utf8.count),
                        UnsafeRawPointer(cfiCStr).assumingMemoryBound(to: UInt8.self),
                        UInt32(cfi.utf8.count),
                        percentage,
                        hlcTs
                    )
                }
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    // MARK: - Bookmarks

    public func addBookmark(_ bookmark: ReaderModels.Bookmark) -> Result<Void, ReaderError> {
        ffiQueue.sync {
            guard let jsonData = try? JSONCoding.encoder.encode(bookmark),
                  let jsonStr = String(data: jsonData, encoding: .utf8) else {
                return .failure(.unknown)
            }

            let code = jsonStr.withCString { cStr in
                reader_add_bookmark(
                    UnsafeRawPointer(cStr).assumingMemoryBound(to: UInt8.self),
                    UInt32(jsonStr.utf8.count)
                )
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    public func deleteBookmark(id: String, hlcTs: UInt64) -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = id.withCString { idCStr in
                reader_delete_bookmark(
                    UnsafeRawPointer(idCStr).assumingMemoryBound(to: UInt8.self),
                    UInt32(id.utf8.count),
                    hlcTs
                )
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    public func listBookmarks(bookId: String) -> Result<[ReaderModels.Bookmark], ReaderError> {
        ffiQueue.sync {
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = bookId.withCString { idCStr in
                reader_list_bookmarks(
                    UnsafeRawPointer(idCStr).assumingMemoryBound(to: UInt8.self),
                    UInt32(bookId.utf8.count),
                    &outPtr,
                    &outLen
                )
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            return decodeJSON(ptr: outPtr, len: outLen)
        }
    }

    // MARK: - Annotations

    public func addAnnotation(_ annotation: ReaderModels.Annotation) -> Result<Void, ReaderError> {
        ffiQueue.sync {
            guard let jsonData = try? JSONCoding.encoder.encode(annotation),
                  let jsonStr = String(data: jsonData, encoding: .utf8) else {
                return .failure(.unknown)
            }

            let code = jsonStr.withCString { cStr in
                reader_add_annotation(
                    UnsafeRawPointer(cStr).assumingMemoryBound(to: UInt8.self),
                    UInt32(jsonStr.utf8.count)
                )
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    public func deleteAnnotation(id: String, hlcTs: UInt64) -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = id.withCString { idCStr in
                reader_delete_annotation(
                    UnsafeRawPointer(idCStr).assumingMemoryBound(to: UInt8.self),
                    UInt32(id.utf8.count),
                    hlcTs
                )
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    public func listAnnotations(bookId: String) -> Result<[ReaderModels.Annotation], ReaderError> {
        ffiQueue.sync {
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = bookId.withCString { idCStr in
                reader_list_annotations(
                    UnsafeRawPointer(idCStr).assumingMemoryBound(to: UInt8.self),
                    UInt32(bookId.utf8.count),
                    &outPtr,
                    &outLen
                )
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            return decodeJSON(ptr: outPtr, len: outLen)
        }
    }

    // MARK: - Private Helpers

    /// Decode JSON from a C pointer+length pair using zero-copy Data wrapping.
    private func decodeJSON<T: Decodable>(ptr: UnsafePointer<UInt8>?, len: UInt32) -> Result<T, ReaderError> {
        guard let ptr = ptr, len > 0 else {
            return .failure(.nullPointer)
        }

        // Zero-copy: wrap the pointer directly without copying bytes.
        // The data is valid as long as the engine's return_buffer is not overwritten.
        let data = Data(bytes: ptr, count: Int(len))

        do {
            let decoded = try JSONCoding.decoder.decode(T.self, from: data)
            return .success(decoded)
        } catch {
            Self.logger.error("JSON decode failed: \(error.localizedDescription)")
            return .failure(.parseFailed)
        }
    }

    // MARK: - Auth Operations

    /// Register a new user account. Returns user ID string.
    public func authRegister(username: String, email: String, password: String) -> Result<String, ReaderError> {
        ffiQueue.sync {
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = username.withCString { userCStr in
                email.withCString { emailCStr in
                    password.withCString { passCStr in
                        reader_auth_register(
                            UnsafeRawPointer(userCStr).assumingMemoryBound(to: UInt8.self),
                            UInt32(username.utf8.count),
                            UnsafeRawPointer(emailCStr).assumingMemoryBound(to: UInt8.self),
                            UInt32(email.utf8.count),
                            UnsafeRawPointer(passCStr).assumingMemoryBound(to: UInt8.self),
                            UInt32(password.utf8.count),
                            &outPtr,
                            &outLen
                        )
                    }
                }
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            guard let ptr = outPtr, outLen > 0 else {
                return .failure(.nullPointer)
            }
            let str = String(bytes: Data(bytes: ptr, count: Int(outLen)), encoding: .utf8) ?? ""
            return .success(str)
        }
    }

    /// Login with credentials. Returns token JSON string.
    public func authLogin(credential: String, password: String) -> Result<String, ReaderError> {
        ffiQueue.sync {
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = credential.withCString { credCStr in
                password.withCString { passCStr in
                    reader_auth_login(
                        UnsafeRawPointer(credCStr).assumingMemoryBound(to: UInt8.self),
                        UInt32(credential.utf8.count),
                        UnsafeRawPointer(passCStr).assumingMemoryBound(to: UInt8.self),
                        UInt32(password.utf8.count),
                        &outPtr,
                        &outLen
                    )
                }
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            guard let ptr = outPtr, outLen > 0 else {
                return .failure(.nullPointer)
            }
            let str = String(bytes: Data(bytes: ptr, count: Int(outLen)), encoding: .utf8) ?? ""
            return .success(str)
        }
    }

    /// Login with credentials and device metadata. Returns token JSON string.
    public func authLoginWithMetadata(
        credential: String,
        password: String,
        deviceName: String?,
        platform: String?
    ) -> Result<String, ReaderError> {
        ffiQueue.sync {
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = credential.withCString { credCStr in
                password.withCString { passCStr in
                    let deviceNamePtr: UnsafePointer<UInt8>?
                    let deviceNameLen: UInt32
                    let platformPtr: UnsafePointer<UInt8>?
                    let platformLen: UInt32

                    // We need to keep the C strings alive for the duration of the call
                    let dnData = deviceName?.utf8CString
                    let pData = platform?.utf8CString

                    if let dn = dnData {
                        deviceNamePtr = dn.withUnsafeBufferPointer { buf in
                            buf.baseAddress?.withMemoryRebound(to: UInt8.self, capacity: dn.count - 1) { $0 }
                        }
                        deviceNameLen = UInt32(deviceName!.utf8.count)
                    } else {
                        deviceNamePtr = nil
                        deviceNameLen = 0
                    }

                    if let p = pData {
                        platformPtr = p.withUnsafeBufferPointer { buf in
                            buf.baseAddress?.withMemoryRebound(to: UInt8.self, capacity: p.count - 1) { $0 }
                        }
                        platformLen = UInt32(platform!.utf8.count)
                    } else {
                        platformPtr = nil
                        platformLen = 0
                    }

                    return reader_auth_login_with_metadata(
                        UnsafeRawPointer(credCStr).assumingMemoryBound(to: UInt8.self),
                        UInt32(credential.utf8.count),
                        UnsafeRawPointer(passCStr).assumingMemoryBound(to: UInt8.self),
                        UInt32(password.utf8.count),
                        deviceNamePtr,
                        deviceNameLen,
                        platformPtr,
                        platformLen,
                        &outPtr,
                        &outLen
                    )
                }
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            guard let ptr = outPtr, outLen > 0 else {
                return .failure(.nullPointer)
            }
            let str = String(bytes: Data(bytes: ptr, count: Int(outLen)), encoding: .utf8) ?? ""
            return .success(str)
        }
    }

    /// Logout the current session.
    public func authLogout() -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = reader_auth_logout()
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    /// Get current auth state (0=LoggedOut, 1=Authenticated, 2=NeedsRefresh, 3=NeedsReLogin).
    public func authGetState() -> Int32 {
        ffiQueue.sync {
            reader_auth_get_state()
        }
    }

    /// Get a valid access token (auto-refreshes if needed).
    public func authGetToken() -> Result<String, ReaderError> {
        ffiQueue.sync {
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = reader_auth_get_token(&outPtr, &outLen)
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            guard let ptr = outPtr, outLen > 0 else {
                return .failure(.nullPointer)
            }
            let str = String(bytes: Data(bytes: ptr, count: Int(outLen)), encoding: .utf8) ?? ""
            return .success(str)
        }
    }

    /// Explicitly refresh the access token. Returns new token JSON.
    public func authRefreshToken() -> Result<String, ReaderError> {
        ffiQueue.sync {
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = reader_auth_refresh_token(&outPtr, &outLen)
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            guard let ptr = outPtr, outLen > 0 else {
                return .failure(.nullPointer)
            }
            let str = String(bytes: Data(bytes: ptr, count: Int(outLen)), encoding: .utf8) ?? ""
            return .success(str)
        }
    }

    /// Change the current user's password.
    public func authChangePassword(oldPassword: String, newPassword: String) -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = oldPassword.withCString { oldCStr in
                newPassword.withCString { newCStr in
                    reader_auth_change_password(
                        UnsafeRawPointer(oldCStr).assumingMemoryBound(to: UInt8.self),
                        UInt32(oldPassword.utf8.count),
                        UnsafeRawPointer(newCStr).assumingMemoryBound(to: UInt8.self),
                        UInt32(newPassword.utf8.count)
                    )
                }
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    /// List devices for the current user. Returns JSON string.
    public func authListDevices() -> Result<String, ReaderError> {
        ffiQueue.sync {
            var outPtr: UnsafePointer<UInt8>?
            var outLen: UInt32 = 0

            let code = reader_auth_list_devices(&outPtr, &outLen)
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }

            guard let ptr = outPtr, outLen > 0 else {
                return .failure(.nullPointer)
            }
            let str = String(bytes: Data(bytes: ptr, count: Int(outLen)), encoding: .utf8) ?? ""
            return .success(str)
        }
    }

    /// Remove a device from the current user's device list.
    public func authRemoveDevice(deviceId: String) -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = deviceId.withCString { idCStr in
                reader_auth_remove_device(
                    UnsafeRawPointer(idCStr).assumingMemoryBound(to: UInt8.self),
                    UInt32(deviceId.utf8.count)
                )
            }

            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    /// Set the auth state change callback.
    public func setAuthCallback(_ callback: (@convention(c) (Int32) -> Void)?) -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = reader_set_auth_callback(callback)
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    // MARK: - Sync Operations

    /// Push local pending operations to the server.
    public func syncPush() -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = reader_sync_push()
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    /// Pull remote operations from the server.
    public func syncPull() -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = reader_sync_pull()
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    /// Perform a full sync (push + pull).
    public func syncFull() -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = reader_sync_full()
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    /// Start the background sync scheduler.
    public func syncStartScheduler() -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = reader_sync_start_scheduler()
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    /// Stop the background sync scheduler.
    public func syncStopScheduler() -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = reader_sync_stop_scheduler()
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }

    /// Set the sync state change callback.
    public func setSyncCallback(_ callback: (@convention(c) (Int32) -> Void)?) -> Result<Void, ReaderError> {
        ffiQueue.sync {
            let code = reader_set_sync_callback(callback)
            if let error = ReaderError.from(code: code) {
                return .failure(error)
            }
            return .success(())
        }
    }
}
