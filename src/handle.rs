use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use tokio::sync::oneshot;
use tracing::{error, info};

use crate::server::{GrpcWebServer, GrpcWebServerOptions};

/// Errors returned by `GrpcWebServerHandle` lifecycle operations.
#[derive(Debug)]
pub enum HandleError {
    /// The lifecycle state lock was poisoned by a previous panic.
    StatePoisoned,
    /// A start request was issued while the server is already running.
    AlreadyRunning,
}

impl fmt::Display for HandleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StatePoisoned => write!(f, "server state lock poisoned"),
            Self::AlreadyRunning => write!(f, "server is already running"),
        }
    }
}

impl std::error::Error for HandleError {}

/// Mutable runtime state for a single FFI server handle.
#[derive(Default)]
struct GrpcWebServerState {
    /// Join handle for the background thread that owns the Tokio runtime.
    thread: Option<thread::JoinHandle<()>>,
    /// One-shot sender used to request graceful shutdown.
    stop_tx: Option<oneshot::Sender<()>>,
}

/// Opaque, heap-allocated handle owned by C/C++ callers.
///
/// Each handle owns an independent server configuration and lifecycle state,
/// allowing multiple server instances to run within the same process. FFI
/// callers receive ownership through `ffi::create` and must release it exactly
/// once through `ffi::destroy`. The handle is safe to operate from multiple
/// threads, but its allocation must not be destroyed while another thread uses
/// its pointer.
pub struct GrpcWebServerHandle {
    /// Per-instance startup options used when `start` is invoked.
    options: GrpcWebServerOptions,
    /// Mutable runtime state guarded for thread-safe FFI access.
    state: Mutex<GrpcWebServerState>,
    /// Per-instance running flag read by `is_running`.
    running: Arc<AtomicBool>,
}

impl GrpcWebServerHandle {
    /// Creates a new reusable server handle.
    ///
    /// The handle does not start any runtime by itself. Call [`Self::start`]
    /// to launch the server thread.
    pub fn new(options: GrpcWebServerOptions) -> GrpcWebServerHandle {
        GrpcWebServerHandle {
            options,
            state: Mutex::new(GrpcWebServerState::default()),
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Starts the server in a background thread and returns without waiting for it.
    ///
    /// # Errors
    /// - `HandleError::StatePoisoned` when the internal state lock is poisoned.
    /// - `HandleError::AlreadyRunning` when a runtime is already active.
    pub fn start(&self) -> Result<(), HandleError> {
        let mut state = self.lock_state()?;

        Self::cleanup_finished_locked(&mut state, &self.running);
        if state.thread.is_some() || self.running.load(Ordering::Acquire) {
            return Err(HandleError::AlreadyRunning);
        }

        let server_options = self.options.clone();
        info!(
            http_address = %server_options.http_address,
            grpc_address = %server_options.grpc_address,
            static_dir = ?server_options.static_dir,
            "Starting grpc-web-server"
        );

        // Creates a pair: sender and receiver. One side sends once, the other receives once.
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let running = Arc::clone(&self.running);
        running.store(true, Ordering::Release);

        // Create a thread to allow `start` to be non-blocking.
        let thread = thread::spawn(move || {
            // Tokio runtime is the async engine (scheduler, timers, and I/O driver).
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    error!("grpc-web-server: failed to create tokio runtime: {err}");
                    running.store(false, Ordering::Release);
                    return;
                }
            };

            // Create a new server.
            let server = match GrpcWebServer::new(server_options) {
                Ok(server) => server,
                Err(err) => {
                    error!("grpc-web-server: failed to create server: {err}");
                    running.store(false, Ordering::Release);
                    return;
                }
            };

            // Resolve once shutdown is requested.
            let shutdown = async move {
                let _ = stop_rx.await;
            };

            // Run the async server to completion on this runtime.
            if let Err(err) = runtime.block_on(server.start(shutdown)) {
                error!("grpc-web-server: server exited with error: {err}");
            }

            running.store(false, Ordering::Release);
        });

        state.stop_tx = Some(stop_tx);
        state.thread = Some(thread);
        Ok(())
    }

    /// Waits for the current background server thread to finish.
    ///
    /// Returns immediately when no thread is active.
    ///
    /// # Errors
    /// - `HandleError::StatePoisoned` when the internal state lock is poisoned.
    pub fn wait(&self) -> Result<(), HandleError> {
        // Release the lock before joining to avoid deadlocks with concurrent stop.
        let thread_handle = {
            let mut state = self.lock_state()?;
            let thread_handle = state.thread.take();
            if thread_handle.is_none() {
                state.stop_tx = None;
            }
            thread_handle
        };

        if let Some(thread_handle) = thread_handle {
            if thread_handle.join().is_err() {
                error!("grpc-web-server: server thread panicked");
            }
        }

        let mut state = self.lock_state()?;
        state.stop_tx = None;
        self.running.store(false, Ordering::Release);
        Ok(())
    }

    /// Returns whether the server runtime is currently marked as running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// Requests graceful shutdown for the running runtime, if any.
    ///
    /// This does not wait for shutdown to complete. Use [`Self::wait`] when the
    /// caller must wait for the server thread to exit.
    ///
    /// # Errors
    /// - `HandleError::StatePoisoned` when the internal state lock is poisoned.
    pub fn stop(&self) -> Result<(), HandleError> {
        let mut state = self.lock_state()?;

        if let Some(stop_tx) = state.stop_tx.take() {
            let _ = stop_tx.send(());
        }

        Ok(())
    }

    /// Joins and clears a finished server thread while holding a handle state lock.
    ///
    /// # Arguments
    /// - `state`: Mutable per-handle state guard.
    /// - `running`: Per-handle running flag.
    fn cleanup_finished_locked(state: &mut GrpcWebServerState, running: &AtomicBool) {
        let Some(handle) = state.thread.take() else {
            return;
        };

        if handle.is_finished() {
            if handle.join().is_err() {
                error!("grpc-web-server: server thread panicked");
            }
            state.stop_tx = None;
            running.store(false, Ordering::Release);
        } else {
            state.thread = Some(handle);
        }
    }

    /// Stops the runtime and joins the background thread.
    ///
    /// This is used by `Drop` to ensure resources are reclaimed even when
    /// callers do not explicitly invoke `stop`/`wait`.
    fn shutdown_and_join(&self) -> Result<(), HandleError> {
        let thread_handle = {
            let mut state = self.lock_state()?;

            if let Some(stop_tx) = state.stop_tx.take() {
                let _ = stop_tx.send(());
            }

            state.thread.take()
        };

        if let Some(thread_handle) = thread_handle {
            if thread_handle.join().is_err() {
                error!("grpc-web-server: server thread panicked");
            }
        }

        self.running.store(false, Ordering::Release);
        Ok(())
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, GrpcWebServerState>, HandleError> {
        self.state.lock().map_err(|_| {
            error!("grpc-web-server: state lock poisoned");
            HandleError::StatePoisoned
        })
    }
}

impl Drop for GrpcWebServerHandle {
    fn drop(&mut self) {
        if let Err(err) = self.shutdown_and_join() {
            error!("grpc-web-server: failed to shutdown handle on drop: {err}");
        }
    }
}
