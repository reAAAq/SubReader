// SyncService — Observable sync state manager.
//
// Bridges Rust sync callbacks to SwiftUI's reactive model.
// Automatically starts/stops the scheduler based on auth state.

import Foundation
import Combine
import CReaderCore

/// Sync state mirroring the Rust SyncState enum.
public enum SyncState: Int32, Sendable {
    case idle = 0
    case syncing = 1
    case error = 2
    case offline = 3
    case dormant = 4
}

private final class SyncServiceCallbackRegistry: @unchecked Sendable {
    private weak var service: SyncService?
    private let lock = NSLock()

    func set(_ service: SyncService?) {
        lock.lock()
        self.service = service
        lock.unlock()
    }

    func get() -> SyncService? {
        lock.lock()
        defer { lock.unlock() }
        return service
    }
}

/// Singleton shared instance for the C callback to update.
private let syncServiceCallbackRegistry = SyncServiceCallbackRegistry()

/// C callback function for sync state changes.
private func syncStateDidChange(_ stateCode: Int32) {
    let newState = SyncState(rawValue: stateCode) ?? .dormant
    guard let service = syncServiceCallbackRegistry.get() else { return }
    DispatchQueue.main.async {
        service.syncState = newState
    }
}

/// Observable sync service.
@MainActor
public final class SyncService: ObservableObject {

    @Published public var syncState: SyncState = .dormant
    @Published public var isLoading = false
    @Published public var autoSyncEnabled: Bool {
        didSet {
            UserDefaults.standard.set(autoSyncEnabled, forKey: "com.subreader.auto-sync")
            if autoSyncEnabled && authService.isLoggedIn {
                startScheduler()
            } else if !autoSyncEnabled {
                stopScheduler()
            }
        }
    }

    private let engine: any ReaderEngineProtocol
    private let authService: AuthService
    private var cancellables = Set<AnyCancellable>()

    public init(engine: any ReaderEngineProtocol, authService: AuthService) {
        self.engine = engine
        self.authService = authService
        self.autoSyncEnabled = UserDefaults.standard.object(forKey: "com.subreader.auto-sync") as? Bool ?? true
        syncServiceCallbackRegistry.set(self)

        // Register the C callback
        let _ = engine.setSyncCallback(syncStateDidChange)

        // Monitor auth state changes to auto-start/stop scheduler
        authService.$authState
            .removeDuplicates()
            .sink { [weak self] state in
                guard let self = self else { return }
                Task { @MainActor in
                    switch state {
                    case .authenticated:
                        if self.autoSyncEnabled {
                            self.startScheduler()
                        }
                    case .loggedOut, .needsReLogin:
                        self.stopScheduler()
                    case .needsRefresh:
                        break // Let the auth layer handle refresh
                    }
                }
            }
            .store(in: &cancellables)
    }

    deinit {
        syncServiceCallbackRegistry.set(nil)
    }

    // MARK: - Public Methods

    /// Trigger an immediate full sync.
    public func syncNow() async -> Result<Void, ReaderError> {
        guard authService.isLoggedIn else {
            return .failure(.authError)
        }

        isLoading = true
        defer { isLoading = false }

        return await Task.detached { [engine] in
            engine.syncFull()
        }.value
    }

    /// Start the background sync scheduler.
    public func startScheduler() {
        let _ = engine.syncStartScheduler()
    }

    /// Stop the background sync scheduler.
    public func stopScheduler() {
        let _ = engine.syncStopScheduler()
    }
}
