// AuthService — Observable authentication state manager.
//
// Bridges Rust auth callbacks to SwiftUI's reactive model.
// All FFI calls are dispatched on RustCore's serial queue.

import Foundation
import Combine
import CReaderCore

/// Authentication state mirroring the Rust AuthState enum.
public enum AuthState: Int32, Sendable {
    case loggedOut = 0
    case authenticated = 1
    case needsRefresh = 2
    case needsReLogin = 3
}

private final class AuthServiceCallbackRegistry: @unchecked Sendable {
    private weak var service: AuthService?
    private let lock = NSLock()

    func set(_ service: AuthService?) {
        lock.lock()
        self.service = service
        lock.unlock()
    }

    func get() -> AuthService? {
        lock.lock()
        defer { lock.unlock() }
        return service
    }
}

/// Singleton shared instance for the C callback to update.
private let authServiceCallbackRegistry = AuthServiceCallbackRegistry()

/// C callback function for auth state changes.
private func authStateDidChange(_ stateCode: Int32) {
    let newState = AuthState(rawValue: stateCode) ?? .loggedOut
    guard let service = authServiceCallbackRegistry.get() else { return }
    DispatchQueue.main.async {
        service.authState = newState
    }
}

/// Observable authentication service.
@MainActor
public final class AuthService: ObservableObject {

    @Published public var authState: AuthState = .loggedOut
    @Published public var isLoading = false
    @Published public var errorMessage: String?

    private let engine: any ReaderEngineProtocol

    public init(engine: any ReaderEngineProtocol) {
        self.engine = engine
        authServiceCallbackRegistry.set(self)

        // Register the C callback
        let _ = engine.setAuthCallback(authStateDidChange)

        // Query initial state
        let stateCode = engine.authGetState()
        self.authState = AuthState(rawValue: stateCode) ?? .loggedOut
    }

    deinit {
        authServiceCallbackRegistry.set(nil)
    }

    /// Whether the user is currently logged in.
    public var isLoggedIn: Bool {
        authState == .authenticated
    }

    // MARK: - Public Methods

    /// Register a new account.
    public func register(username: String, email: String, password: String) async -> Result<String, ReaderError> {
        isLoading = true
        errorMessage = nil
        defer { isLoading = false }

        let result = await Task.detached { [engine] in
            engine.authRegister(username: username, email: email, password: password)
        }.value

        if case .failure(let error) = result {
            errorMessage = error.localizedDescription
        }
        return result
    }

    /// Login with credentials and device metadata.
    public func login(credential: String, password: String) async -> Result<String, ReaderError> {
        isLoading = true
        errorMessage = nil
        defer { isLoading = false }

        let deviceName = Host.current().localizedName ?? "Mac"
        let platform = "macOS"

        let result = await Task.detached { [engine] in
            engine.authLoginWithMetadata(
                credential: credential,
                password: password,
                deviceName: deviceName,
                platform: platform
            )
        }.value

        if case .failure(let error) = result {
            errorMessage = error.localizedDescription
        }
        return result
    }

    /// Logout the current session.
    public func logout() async -> Result<Void, ReaderError> {
        isLoading = true
        errorMessage = nil
        defer { isLoading = false }

        let result = await Task.detached { [engine] in
            engine.authLogout()
        }.value

        if case .failure(let error) = result {
            errorMessage = error.localizedDescription
        }
        return result
    }

    /// Change password.
    public func changePassword(oldPassword: String, newPassword: String) async -> Result<Void, ReaderError> {
        isLoading = true
        errorMessage = nil
        defer { isLoading = false }

        let result = await Task.detached { [engine] in
            engine.authChangePassword(oldPassword: oldPassword, newPassword: newPassword)
        }.value

        if case .failure(let error) = result {
            errorMessage = error.localizedDescription
        }
        return result
    }

    /// List devices.
    public func listDevices() async -> Result<String, ReaderError> {
        await Task.detached { [engine] in
            engine.authListDevices()
        }.value
    }

    /// Remove a device.
    public func removeDevice(deviceId: String) async -> Result<Void, ReaderError> {
        await Task.detached { [engine] in
            engine.authRemoveDevice(deviceId: deviceId)
        }.value
    }
}
