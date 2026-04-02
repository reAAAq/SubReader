//! Sync scheduler — manages periodic sync, debounced push, and lifecycle.
//!
//! The scheduler runs sync operations on a background tokio task,
//! with a 30-second polling interval and 2-second debounced push.

use crate::engine::{SyncEngine, SyncStorage, SyncTransportAdapter};
use crate::SyncError;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Token provider trait for obtaining the latest valid auth token.
pub trait TokenProvider: Send + Sync + 'static {
    /// Get the current valid auth token.
    /// Returns None if not authenticated.
    fn get_token(&self) -> Option<String>;
}

/// Sync scheduler state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    /// Idle, waiting for next sync cycle.
    Idle,
    /// Currently syncing.
    Syncing,
    /// Last sync encountered an error.
    Error,
    /// Network is offline.
    Offline,
    /// User is not logged in, scheduler is dormant.
    Dormant,
}

/// Sync state change callback type.
pub type StateCallback = Box<dyn Fn(SyncState) + Send + Sync>;

/// Commands that can be sent to the scheduler.
enum SchedulerCommand {
    /// Trigger an immediate full sync (push + pull).
    SyncNow,
    /// Trigger a debounced push.
    TriggerPush,
    /// Stop the scheduler.
    Stop,
}

/// Sync scheduler that manages the lifecycle of sync operations.
pub struct SyncScheduler<S: SyncStorage + 'static, T: SyncTransportAdapter + 'static> {
    engine: Arc<SyncEngine<S, T>>,
    state: Arc<Mutex<SyncState>>,
    command_tx: Option<mpsc::Sender<SchedulerCommand>>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
    state_callback: Arc<Mutex<Option<StateCallback>>>,
}

impl<S: SyncStorage + 'static, T: SyncTransportAdapter + 'static> SyncScheduler<S, T> {
    /// Create a new sync scheduler (not yet started).
    pub fn new(engine: SyncEngine<S, T>) -> Self {
        Self {
            engine: Arc::new(engine),
            state: Arc::new(Mutex::new(SyncState::Dormant)),
            command_tx: None,
            task_handle: None,
            state_callback: Arc::new(Mutex::new(None)),
        }
    }

    /// Set a callback for sync state changes.
    pub async fn set_state_callback(&self, callback: StateCallback) {
        *self.state_callback.lock().await = Some(callback);
    }

    /// Get the current sync state.
    pub async fn state(&self) -> SyncState {
        *self.state.lock().await
    }

    /// Start the scheduler with periodic polling.
    ///
    /// This spawns a background tokio task that:
    /// - Immediately performs a full sync
    /// - Polls every 30 seconds
    /// - Responds to manual sync/push commands
    /// - Fetches the latest auth token before each operation
    pub fn start(&mut self, token_provider: Arc<dyn TokenProvider>) {
        if self.command_tx.is_some() {
            tracing::debug!("Sync scheduler already running, ignoring duplicate start");
            return;
        }

        let (tx, mut rx) = mpsc::channel::<SchedulerCommand>(32);
        self.command_tx = Some(tx);

        let engine = Arc::clone(&self.engine);
        let state = Arc::clone(&self.state);
        let state_callback = Arc::clone(&self.state_callback);
        let sync_mutex = Arc::new(Mutex::new(()));

        let handle = tokio::spawn(async move {
            // Set state to idle
            Self::set_state_inner(&state, &state_callback, SyncState::Idle).await;

            // Immediately perform a full sync
            if let Some(token) = token_provider.get_token() {
                Self::do_sync(&engine, &state, &state_callback, &sync_mutex, &token).await;
            } else {
                tracing::warn!("No auth token available at scheduler start, entering dormant state");
                Self::set_state_inner(&state, &state_callback, SyncState::Dormant).await;
            }

            let mut poll_interval = tokio::time::interval(std::time::Duration::from_secs(30));
            poll_interval.tick().await; // Skip the first immediate tick

            let mut push_debounce: Option<tokio::time::Instant> = None;

            loop {
                tokio::select! {
                    _ = poll_interval.tick() => {
                        match token_provider.get_token() {
                            Some(token) => {
                                Self::do_sync(&engine, &state, &state_callback, &sync_mutex, &token).await;
                            }
                            None => {
                                tracing::warn!("No auth token available for periodic sync, skipping");
                                Self::set_state_inner(&state, &state_callback, SyncState::Dormant).await;
                            }
                        }
                    }
                    cmd = rx.recv() => {
                        match cmd {
                            Some(SchedulerCommand::SyncNow) => {
                                match token_provider.get_token() {
                                    Some(token) => {
                                        Self::do_sync(&engine, &state, &state_callback, &sync_mutex, &token).await;
                                    }
                                    None => {
                                        tracing::warn!("No auth token available for manual sync");
                                        Self::set_state_inner(&state, &state_callback, SyncState::Dormant).await;
                                    }
                                }
                            }
                            Some(SchedulerCommand::TriggerPush) => {
                                // Debounce: wait 2 seconds before pushing
                                push_debounce = Some(tokio::time::Instant::now() + std::time::Duration::from_secs(2));
                            }
                            Some(SchedulerCommand::Stop) | None => {
                                tracing::info!("Sync scheduler stopping");
                                // Final push before stopping
                                if let Some(token) = token_provider.get_token() {
                                    Self::do_push(&engine, &state, &state_callback, &sync_mutex, &token).await;
                                }
                                Self::set_state_inner(&state, &state_callback, SyncState::Dormant).await;
                                break;
                            }
                        }
                    }
                    _ = async {
                        if let Some(deadline) = push_debounce {
                            tokio::time::sleep_until(deadline).await;
                        } else {
                            // Sleep forever if no debounce pending
                            std::future::pending::<()>().await;
                        }
                    } => {
                        push_debounce = None;
                        match token_provider.get_token() {
                            Some(token) => {
                                Self::do_push(&engine, &state, &state_callback, &sync_mutex, &token).await;
                            }
                            None => {
                                tracing::warn!("No auth token available for debounced push");
                                Self::set_state_inner(&state, &state_callback, SyncState::Dormant).await;
                            }
                        }
                    }
                }
            }
        });

        self.task_handle = Some(handle);
    }

    /// Stop the scheduler.
    pub async fn stop(&mut self) {
        if let Some(tx) = self.command_tx.take() {
            let _ = tx.send(SchedulerCommand::Stop).await;
        }

        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        } else {
            Self::set_state_inner(&self.state, &self.state_callback, SyncState::Dormant).await;
        }
    }

    /// Trigger an immediate full sync (push + pull).
    pub async fn trigger_now(&self) -> Result<(), SyncError> {
        if let Some(tx) = &self.command_tx {
            tx.send(SchedulerCommand::SyncNow)
                .await
                .map_err(|_| SyncError::Transport("Scheduler not running".to_string()))?;
        }
        Ok(())
    }

    /// Trigger a debounced push (2-second delay).
    pub async fn trigger_push(&self) -> Result<(), SyncError> {
        if let Some(tx) = &self.command_tx {
            tx.send(SchedulerCommand::TriggerPush)
                .await
                .map_err(|_| SyncError::Transport("Scheduler not running".to_string()))?;
        }
        Ok(())
    }

    // ─── Internal helpers ───────────────────────────────────────────────────

    async fn do_sync(
        engine: &Arc<SyncEngine<S, T>>,
        state: &Arc<Mutex<SyncState>>,
        state_callback: &Arc<Mutex<Option<StateCallback>>>,
        sync_mutex: &Arc<Mutex<()>>,
        auth_token: &str,
    ) {
        // Prevent concurrent syncs
        let _guard = match sync_mutex.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                tracing::debug!("Sync already in progress, skipping");
                // Notify via callback so callers know the request was dropped
                Self::set_state_inner(state, state_callback, SyncState::Syncing).await;
                return;
            }
        };

        Self::set_state_inner(state, state_callback, SyncState::Syncing).await;

        match engine.sync(auth_token).await {
            Ok((pushed, pulled)) => {
                tracing::info!(pushed = pushed, pulled = pulled, "Sync cycle completed");
                Self::set_state_inner(state, state_callback, SyncState::Idle).await;
            }
            Err(e) => {
                tracing::error!(error = %e, "Sync cycle failed");
                let new_state = match &e {
                    SyncError::NotAuthenticated => {
                        tracing::error!("Authentication failed during sync — token may be expired or revoked");
                        SyncState::Dormant
                    }
                    SyncError::Transport(_) => SyncState::Offline,
                    _ => SyncState::Error,
                };
                Self::set_state_inner(state, state_callback, new_state).await;
            }
        }
    }

    async fn do_push(
        engine: &Arc<SyncEngine<S, T>>,
        state: &Arc<Mutex<SyncState>>,
        state_callback: &Arc<Mutex<Option<StateCallback>>>,
        sync_mutex: &Arc<Mutex<()>>,
        auth_token: &str,
    ) {
        let _guard = match sync_mutex.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                tracing::debug!("Sync already in progress, skipping push");
                // Notify via callback so callers know the request was dropped
                Self::set_state_inner(state, state_callback, SyncState::Syncing).await;
                return;
            }
        };

        Self::set_state_inner(state, state_callback, SyncState::Syncing).await;

        match engine.push_pending(auth_token).await {
            Ok(count) => {
                tracing::debug!(pushed = count, "Push completed");
                Self::set_state_inner(state, state_callback, SyncState::Idle).await;
            }
            Err(e) => {
                tracing::error!(error = %e, "Push failed");
                Self::set_state_inner(state, state_callback, SyncState::Error).await;
            }
        }
    }

    async fn set_state_inner(
        state: &Arc<Mutex<SyncState>>,
        state_callback: &Arc<Mutex<Option<StateCallback>>>,
        new_state: SyncState,
    ) {
        *state.lock().await = new_state;
        if let Some(ref cb) = *state_callback.lock().await {
            cb(new_state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        PullResult, PushOpPayload, RemoteOp, SyncStorage, SyncTransportAdapter, UnsyncedOp,
    };
    use crate::SyncError;
    use std::collections::HashMap;

    // ─── SyncState Tests ─────────────────────────────────────────────────

    #[test]
    fn test_sync_state_equality() {
        assert_eq!(SyncState::Idle, SyncState::Idle);
        assert_eq!(SyncState::Syncing, SyncState::Syncing);
        assert_eq!(SyncState::Error, SyncState::Error);
        assert_eq!(SyncState::Offline, SyncState::Offline);
        assert_eq!(SyncState::Dormant, SyncState::Dormant);
        assert_ne!(SyncState::Idle, SyncState::Syncing);
    }

    #[test]
    fn test_sync_state_debug() {
        assert_eq!(format!("{:?}", SyncState::Idle), "Idle");
        assert_eq!(format!("{:?}", SyncState::Syncing), "Syncing");
        assert_eq!(format!("{:?}", SyncState::Error), "Error");
        assert_eq!(format!("{:?}", SyncState::Offline), "Offline");
        assert_eq!(format!("{:?}", SyncState::Dormant), "Dormant");
    }

    #[test]
    fn test_sync_state_clone() {
        let state = SyncState::Syncing;
        let cloned = state;
        assert_eq!(state, cloned);
    }

    // ─── Mock implementations for scheduler tests ────────────────────────

    struct MockStorage {
        meta: std::sync::Mutex<HashMap<String, String>>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                meta: std::sync::Mutex::new(HashMap::new()),
            }
        }
    }

    impl SyncStorage for MockStorage {
        fn get_unsynced_ops(&self) -> Result<Vec<UnsyncedOp>, SyncError> {
            Ok(vec![])
        }
        fn mark_ops_synced(&self, _local_ids: &[i64]) -> Result<(), SyncError> {
            Ok(())
        }
        fn get_sync_meta(&self, key: &str) -> Result<Option<String>, SyncError> {
            Ok(self.meta.lock().unwrap().get(key).cloned())
        }
        fn set_sync_meta(&self, key: &str, value: &str) -> Result<(), SyncError> {
            self.meta
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }
        fn apply_remote_op(&self, _op: &RemoteOp) -> Result<bool, SyncError> {
            Ok(true)
        }
    }

    struct MockTransport;

    impl SyncTransportAdapter for MockTransport {
        async fn push_ops(
            &self,
            _ops: &[PushOpPayload],
            _auth_token: &str,
        ) -> Result<usize, SyncError> {
            Ok(0)
        }
        async fn pull_ops(
            &self,
            _cursor: i64,
            _limit: i64,
            _auth_token: &str,
        ) -> Result<PullResult, SyncError> {
            Ok(PullResult {
                operations: vec![],
                next_cursor: 0,
                has_more: false,
            })
        }
    }

    struct MockTokenProvider {
        token: std::sync::Mutex<Option<String>>,
    }

    impl MockTokenProvider {
        fn with_token(token: &str) -> Arc<Self> {
            Arc::new(Self {
                token: std::sync::Mutex::new(Some(token.to_string())),
            })
        }
    }

    impl TokenProvider for MockTokenProvider {
        fn get_token(&self) -> Option<String> {
            self.token.lock().unwrap().clone()
        }
    }

    // ─── Scheduler Tests ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_scheduler_initial_state_is_dormant() {
        let storage = MockStorage::new();
        let transport = MockTransport;
        let engine = crate::engine::SyncEngine::new(storage, transport, "dev-1".into(), 1);
        let scheduler = SyncScheduler::new(engine);

        assert_eq!(scheduler.state().await, SyncState::Dormant);
    }

    #[tokio::test]
    async fn test_scheduler_start_transitions_to_idle() {
        let storage = MockStorage::new();
        let transport = MockTransport;
        let engine = crate::engine::SyncEngine::new(storage, transport, "dev-1".into(), 1);
        let mut scheduler = SyncScheduler::new(engine);

        scheduler.start(MockTokenProvider::with_token("test-token"));

        // Give the background task time to start and sync
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let state = scheduler.state().await;
        assert!(
            state == SyncState::Idle || state == SyncState::Syncing,
            "Expected Idle or Syncing, got {:?}",
            state
        );
    }

    #[tokio::test]
    async fn test_scheduler_stop() {
        let storage = MockStorage::new();
        let transport = MockTransport;
        let engine = crate::engine::SyncEngine::new(storage, transport, "dev-1".into(), 1);
        let mut scheduler = SyncScheduler::new(engine);

        scheduler.start(MockTokenProvider::with_token("test-token"));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        scheduler.stop().await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert_eq!(scheduler.state().await, SyncState::Dormant);
    }

    #[tokio::test]
    async fn test_scheduler_trigger_now() {
        let storage = MockStorage::new();
        let transport = MockTransport;
        let engine = crate::engine::SyncEngine::new(storage, transport, "dev-1".into(), 1);
        let mut scheduler = SyncScheduler::new(engine);

        scheduler.start(MockTokenProvider::with_token("test-token"));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let result = scheduler.trigger_now().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_scheduler_trigger_push() {
        let storage = MockStorage::new();
        let transport = MockTransport;
        let engine = crate::engine::SyncEngine::new(storage, transport, "dev-1".into(), 1);
        let mut scheduler = SyncScheduler::new(engine);

        scheduler.start(MockTokenProvider::with_token("test-token"));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let result = scheduler.trigger_push().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_scheduler_trigger_without_start_is_noop() {
        let storage = MockStorage::new();
        let transport = MockTransport;
        let engine = crate::engine::SyncEngine::new(storage, transport, "dev-1".into(), 1);
        let scheduler = SyncScheduler::new(engine);

        // Should not error, just no-op
        let result = scheduler.trigger_now().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_scheduler_state_callback() {
        let storage = MockStorage::new();
        let transport = MockTransport;
        let engine = crate::engine::SyncEngine::new(storage, transport, "dev-1".into(), 1);
        let mut scheduler = SyncScheduler::new(engine);

        let states = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let states_clone = states.clone();

        scheduler
            .set_state_callback(Box::new(move |state| {
                states_clone.lock().unwrap().push(state);
            }))
            .await;

        scheduler.start(MockTokenProvider::with_token("test-token"));
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let recorded = states.lock().unwrap();
        // Should have at least recorded Idle and Syncing transitions
        assert!(!recorded.is_empty(), "Expected state transitions to be recorded");
    }
}
