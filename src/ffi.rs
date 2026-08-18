use std::{
    ffi::{CStr, c_char},
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use tokio::sync::oneshot;
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, fmt};

use crate::server::{Server, ServerOptions};

#[derive(Debug, Clone)]
#[repr(C)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

#[derive(Debug, Clone)]
#[repr(C)]
pub struct GrpcWebProxyOptions {
    /// The address to host the HTTP/1.1 proxy/web server on.
    pub http_address: *const c_char,
    /// The address of the gRPC server to forwarded the requests to and from
    pub grpc_address: *const c_char,
    /// The directory of the static web files to host
    pub static_dir: *const c_char,
    /// The path to the CA cert used to generate the gRPC server and proxy certificates and keys. Required for TLS.
    pub grpc_ca_cert: *const c_char,
    /// The path to the gRPC proxy private key. Required for mTLS
    pub grpc_proxy_key: *const c_char,
    /// The path to the gRPC proxy certification. Required for mTLS
    pub grpc_proxy_cert: *const c_char,
    /// The path to the HTTP private key. Required for HTTP TLS
    pub http_key: *const c_char,
    /// The path to the HTTP certification. Required for HTTP TLS
    pub http_cert: *const c_char,
    /// Logging verbosity level
    pub log_level: LogLevel,
}

impl GrpcWebProxyOptions {
    /// Converts a nullable C string pointer into an owned Rust string.
    ///
    /// # Arguments
    /// - `ptr`: Nullable pointer to a NUL-terminated C string.
    ///
    /// # Returns
    /// - None when `ptr` is null.
    /// - `Some(String)` when conversion succeeds.
    ///
    /// # Errors
    /// - The bytes pointed to by `ptr` are not valid UTF-8.
    fn c_char_to_string(ptr: *const c_char) -> Result<Option<String>, Box<dyn std::error::Error>> {
        if ptr.is_null() {
            return Ok(None);
        }
        unsafe {
            let c_str = CStr::from_ptr(ptr);
            let rust_str = c_str.to_str()?;
            Ok(Some(rust_str.to_owned()))
        }
    }
}

impl TryInto<ServerOptions> for GrpcWebProxyOptions {
    type Error = Box<dyn std::error::Error>;

    /// Converts FFI options into Rust server options.
    ///
    /// # Returns
    /// `ServerOptions` when required fields and string conversions are valid.
    ///
    /// # Errors
    /// - `http_address` is not provided.
    /// - `grpc_address` is not provided.
    /// - Any provided C string is not valid UTF-8.
    fn try_into(self) -> Result<ServerOptions, Self::Error> {
        let server_options = ServerOptions {
            http_address: Self::c_char_to_string(self.http_address)?
                .ok_or(format!("http_address must be defined"))?,
            grpc_address: Self::c_char_to_string(self.grpc_address)?
                .ok_or(format!("grpc_address must be defined"))?,
            static_dir: Self::c_char_to_string(self.static_dir)?,
            grpc_ca_cert: Self::c_char_to_string(self.grpc_ca_cert)
                .map_err(|err| format!("Err with grpc_ca_cert: {err}"))?
                .map(PathBuf::from),
            grpc_proxy_key: Self::c_char_to_string(self.grpc_proxy_key)
                .map_err(|err| format!("Err with grpc_proxy_key: {err}"))?
                .map(PathBuf::from),
            grpc_proxy_cert: Self::c_char_to_string(self.grpc_proxy_cert)
                .map_err(|err| format!("Err with grpc_proxy_cert: {err}"))?
                .map(PathBuf::from),
            http_key: Self::c_char_to_string(self.http_key)
                .map_err(|err| format!("Err with http_key: {err}"))?
                .map(PathBuf::from),
            http_cert: Self::c_char_to_string(self.http_cert)
                .map_err(|err| format!("Err with http_cert: {err}"))?
                .map(PathBuf::from),
        };
        Ok(server_options)
    }
}

#[derive(Default)]
struct FfiServerState {
    /// Join handle for the background thread that owns the Tokio runtime.
    thread: Option<thread::JoinHandle<()>>,
    /// One-shot sender used to request graceful shutdown.
    stop_tx: Option<oneshot::Sender<()>>,
}

/// Lazily initialized process-wide server state used by FFI entry points.
static SERVER_STATE: OnceLock<Mutex<FfiServerState>> = OnceLock::new();
/// Fast atomic flag read by `is_running` to report runtime state across threads.
static SERVER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Returns the singleton mutable state container used by FFI lifecycle functions.
///
/// # Returns
/// - A reference to the global mutex containing server lifecycle state.
fn server_state() -> &'static Mutex<FfiServerState> {
    SERVER_STATE.get_or_init(|| Mutex::new(FfiServerState::default()))
}

/// Joins and clears a finished server thread while holding the state lock.
///
/// # Arguments
/// - `state`: Mutable global FFI state guard.
fn cleanup_finished_locked(state: &mut FfiServerState) {
    let Some(handle) = state.thread.take() else {
        return;
    };

    if handle.is_finished() {
        if handle.join().is_err() {
            error!("grpc-web-server: server thread panicked");
        }
        state.stop_tx = None;
        SERVER_RUNNING.store(false, Ordering::Release);
    } else {
        state.thread = Some(handle);
    }
}

/// Starts the proxy in a background thread and returns immediately.
///
/// # Arguments
/// - `options`: FFI server options passed from C/C++.
///
/// # Panics
/// - This function catches unwinds with `catch_unwind` to avoid unwinding across the FFI boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn start(options: GrpcWebProxyOptions) {
    // prevents Rust panics from unwinding across C FFI boundaries. Unwinding across FFI is undefined behaviour, so this
    // is protective.
    let _ = std::panic::catch_unwind(|| {
        let filter = EnvFilter::new(format!(
            "grpc_web_server={},tower_http=info",
            options.clone().log_level.as_str()
        ));
        fmt().with_env_filter(filter).init();

        let server_options: ServerOptions = match options.try_into() {
            Ok(options) => options,
            Err(err) => {
                error!("grpc-web-server: invalid options: {err}");
                return;
            }
        };

        info!(
            http_address = %server_options.http_address,
            grpc_address = %server_options.grpc_address,
            static_dir = ?server_options.static_dir,
            "Starting grpc-web-server"
        );

        let state = server_state();
        let mut state = match state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                error!("grpc-web-server: state lock poisoned");
                poisoned.into_inner()
            }
        };

        cleanup_finished_locked(&mut state);
        if state.thread.is_some() || SERVER_RUNNING.load(Ordering::Acquire) {
            return;
        }

        // Creates a pair: sender and receiver. One side sender once, the other receives once. We create `(stop_tx,
        // stop_rx)`, store `stop_tx` globally, and await `stop_rx` in async shutdown logic
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        SERVER_RUNNING.store(true, Ordering::Release);

        // Create a thread to allow `start` to be non-blocking
        let thread = thread::spawn(move || {
            // Tokio runtime is the async engine (scheduler, timers, I/O driver). Our background thread creates one
            // runtime and then runs async server code with `block_on`. THis is the bridge between sync FFI entrypoints
            // and async server internals
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    error!("grpc-web-server: failed to create tokio runtime: {err}");
                    SERVER_RUNNING.store(false, Ordering::Release);
                    return;
                }
            };

            // Create a new server
            let server = match Server::new(server_options) {
                Ok(server) => server,
                Err(err) => {
                    error!("grpc-web-server: failed to create server: {err}");
                    SERVER_RUNNING.store(false, Ordering::Release);
                    return;
                }
            };

            // This creates a future that completes when `stop_tx` sends (or sender is dropped). `move` transfers the
            // ownership of `stop_rx` into this future. THe server waits on this future as its shutdown trigger.
            let shutdown = async move {
                let _ = stop_rx.await;
            };

            // Runs a future to completion on the Tokio runtime. This runs the given future on the current thread,
            // blocking until it is complete, and yielding its resolved result. Any tasks or timers which the future
            // spawns internally will be executed on the runtime.
            if let Err(err) = runtime.block_on(server.start(shutdown)) {
                error!("grpc-web-server: server exited with error: {err}");
            }

            // Set to false as the server clearly failed to launch properly
            SERVER_RUNNING.store(false, Ordering::Release);
        });

        state.stop_tx = Some(stop_tx);
        state.thread = Some(thread);
    });
}

/// Blocks until the current background server thread exits.
///
/// # Panics
/// - This function catches unwinds with `catch_unwind` to avoid unwinding across the FFI boundary.
#[unsafe(no_mangle)]
pub extern "C" fn wait() {
    // prevents Rust panics from unwinding across C FFI boundaries. Unwinding across FFI is undefined behaviour, so this
    // is protective.
    let _ = std::panic::catch_unwind(|| {
        // Get the server thread handle
        let handle = {
            let state = server_state();
            let mut state = match state.lock() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    error!("grpc-web-server: state lock poisoned");
                    poisoned.into_inner()
                }
            };
            let handle = state.thread.take();
            if handle.is_none() {
                state.stop_tx = None;
            }
            handle
        };

        // Await the thread join to wait until the server finishes executing
        if let Some(handle) = handle {
            if handle.join().is_err() {
                error!("grpc-web-server: server thread panicked");
            }
        }

        // Clean up
        let state = server_state();
        let mut state = match state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.stop_tx = None;
        SERVER_RUNNING.store(false, Ordering::Release);
    });
}

/// Returns `true` if the proxy runtime is currently marked as running.
///
/// # Returns
/// - `true` when the runtime is marked running, otherwise `false`.
///
/// # Panics
/// - This function does not intentionally panic.
#[unsafe(no_mangle)]
pub extern "C" fn is_running() -> bool {
    SERVER_RUNNING.load(Ordering::Acquire)
}

/// Requests graceful shutdown for the running server, if any.
///
/// # Panics
/// - This function catches unwinds with `catch_unwind` to avoid unwinding across the FFI boundary.
#[unsafe(no_mangle)]
pub extern "C" fn stop() {
    // prevents Rust panics from unwinding across C FFI boundaries. Unwinding across FFI is undefined behaviour, so this
    // is protective.
    let _ = std::panic::catch_unwind(|| {
        let state = server_state();
        let mut state = match state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                error!("grpc-web-server: state lock poisoned");
                poisoned.into_inner()
            }
        };

        if let Some(stop_tx) = state.stop_tx.take() {
            let _ = stop_tx.send(());
        }
    });
}
