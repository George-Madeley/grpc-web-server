//! C-compatible API for creating and controlling gRPC-Web server instances.
//!
//! `create` allocates an opaque [`handle::GrpcWebServerHandle`] on the heap and
//! transfers ownership to the caller. The returned pointer remains valid until
//! it is passed to `destroy`, which is the only function that frees it. All
//! other exported functions borrow the handle and do not take ownership.
//!
//! A caller must not use a handle after `destroy`, destroy it more than once,
//! or call `destroy` concurrently with another operation using that handle.

use std::{
    ffi::{CStr, c_char},
    path::PathBuf,
    sync::Once,
};

use tracing::error;
use tracing_subscriber::{EnvFilter, fmt};

use crate::{handle, server};

/// Logging verbosity for a server handle.
///
/// The first successful call to [`create`] initializes process-wide tracing
/// with this level. Later calls leave that tracing subscriber unchanged.
#[derive(Debug, Clone)]
#[repr(C)]
pub enum LogLevel {
    /// Show only error events.
    Error,
    /// Show warning and error events.
    Warn,
    /// Show informational, warning, and error events.
    Info,
    /// Show debug events and all less verbose events.
    Debug,
    /// Show every event, including trace events.
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

/// C-compatible options used to configure one gRPC-Web server handle.
///
/// Each non-null string pointer must refer to a valid, NUL-terminated UTF-8
/// string for the duration of [`create`]. `create` copies each string, so the
/// caller may release the original strings after it returns. `http_address`
/// and `grpc_address` are required; all other string fields are optional.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct GrpcWebServerOptions {
    /// The address to host the HTTP/1.1 proxy/web server on.
    pub http_address: *const c_char,
    /// Address of the upstream gRPC server that receives proxied requests.
    pub grpc_address: *const c_char,
    /// Directory containing static web files to serve.
    pub static_dir: *const c_char,
    /// CA certificate path used to verify the upstream gRPC server for mTLS.
    pub grpc_ca_cert: *const c_char,
    /// Client private-key path used for upstream gRPC mTLS.
    pub grpc_proxy_key: *const c_char,
    /// Client certificate path used for upstream gRPC mTLS.
    pub grpc_proxy_cert: *const c_char,
    /// Private-key path used to enable TLS for the HTTP server.
    pub http_key: *const c_char,
    /// Certificate path used to enable TLS for the HTTP server.
    pub http_cert: *const c_char,
    /// Process-wide tracing verbosity selected by the first created handle.
    pub log_level: LogLevel,
}

impl GrpcWebServerOptions {
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
    ///
    /// # Safety
    /// When non-null, `ptr` must point to a valid NUL-terminated C string.
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

impl TryInto<server::GrpcWebServerOptions> for GrpcWebServerOptions {
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
    fn try_into(self) -> Result<server::GrpcWebServerOptions, Self::Error> {
        let server_options = server::GrpcWebServerOptions {
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

/// Process-wide one-time tracing initialization for all server handles.
static TRACING_INIT: Once = Once::new();

/// Initializes tracing once for the process.
///
/// The first created handle decides the initial log level. Later calls keep the
/// existing subscriber and ignore new levels.
fn init_tracing_once(level: LogLevel) {
    TRACING_INIT.call_once(|| {
        let filter = EnvFilter::new(format!(
            "grpc_web_server={},tower_http=info",
            level.as_str()
        ));
        let _ = fmt().with_env_filter(filter).try_init();
    });
}

/// Creates a heap-allocated server handle from FFI options.
///
/// The returned non-null pointer transfers ownership of the allocation to the
/// caller. Pass it to `start`, `stop`, `wait`, and `is_running` to operate on
/// the handle, then pass it exactly once to `destroy` to release it. The
/// handle is configured but not started by this call.
///
/// # Arguments
/// - `options`: FFI server options passed from C/C++.
///
/// # Returns
/// - Non-null owning pointer to a newly allocated server handle on success.
/// - Null pointer on option validation or allocation failure.
///
/// # Safety
/// - Any non-null C string pointer in `options` must be valid, NUL-terminated,
///   and contain UTF-8.
/// - `options.log_level` must contain a valid [`LogLevel`] discriminant.
///
/// # Panics
/// - This function catches unwinds to avoid unwinding across the FFI boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn create(options: GrpcWebServerOptions) -> *mut handle::GrpcWebServerHandle {
    std::panic::catch_unwind(|| {
        init_tracing_once(options.log_level.clone());

        let server_options = match options.try_into() {
            Ok(options) => options,
            Err(err) => {
                error!("grpc-web-server: invalid options: {err}");
                return std::ptr::null_mut();
            }
        };

        let handle = handle::GrpcWebServerHandle::new(server_options);
        Box::into_raw(Box::new(handle))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Starts a server handle in a background thread.
///
/// # Arguments
/// - `handle`: Pointer returned by `create`.
///
/// # Returns
/// - `true` when a new background runtime is started.
/// - `false` when `handle` is null, already running, or startup fails.
///
/// # Safety
/// - `handle` must be a non-null pointer returned by `create` that has not
///   been passed to `destroy`.
/// - No thread may call `destroy` while this function is using `handle`.
///
/// # Panics
/// - This function catches unwinds to avoid unwinding across the FFI boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn start(handle: *mut handle::GrpcWebServerHandle) -> bool {
    std::panic::catch_unwind(|| {
        if handle.is_null() {
            return false;
        }

        let handle = unsafe { &*handle };
        if let Err(err) = handle.start() {
            error!("grpc-web-server: failed to start handle: {err}");
            return false;
        }

        true
    })
    .unwrap_or(false)
}

/// Blocks until the background server thread for `handle` exits.
///
/// If the server is running, this call waits for completion. If it is not
/// running, the function returns `true` immediately.
///
/// # Arguments
/// - `handle`: Pointer returned by `create`.
///
/// # Returns
/// - `true` when the call completes successfully.
/// - `false` when `handle` is null.
///
/// # Safety
/// - `handle` must be a non-null pointer returned by `create` that has not
///   been passed to `destroy`.
/// - No thread may call `destroy` while this function is using `handle`.
///
/// # Panics
/// - This function catches unwinds to avoid unwinding across the FFI boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wait(handle: *mut handle::GrpcWebServerHandle) -> bool {
    std::panic::catch_unwind(|| {
        if handle.is_null() {
            return false;
        }

        let handle = unsafe { &*handle };
        if let Err(err) = handle.wait() {
            error!("grpc-web-server: failed to wait for handle: {err}");
            return false;
        }

        true
    })
    .unwrap_or(false)
}

/// Returns `true` if `handle` is currently marked as running.
///
/// # Arguments
/// - `handle`: Pointer returned by `create`.
///
/// # Returns
/// - `true` when the handle runtime is marked running, otherwise `false`.
/// - `false` when `handle` is null.
///
/// # Safety
/// - `handle` must be a non-null pointer returned by `create` that has not
///   been passed to `destroy`.
/// - No thread may call `destroy` while this function is using `handle`.
///
/// # Panics
/// - This function does not intentionally panic.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn is_running(handle: *const handle::GrpcWebServerHandle) -> bool {
    if handle.is_null() {
        return false;
    }

    let handle = unsafe { &*handle };
    handle.is_running()
}

/// Requests graceful shutdown for a running server handle, if any.
///
/// # Arguments
/// - `handle`: Pointer returned by `create`.
///
/// # Returns
/// - `true` when the stop signal is sent or no running server exists.
/// - `false` when `handle` is null.
///
/// # Safety
/// - `handle` must be a non-null pointer returned by `create` that has not
///   been passed to `destroy`.
/// - No thread may call `destroy` while this function is using `handle`.
///
/// # Panics
/// - This function catches unwinds to avoid unwinding across the FFI boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stop(handle: *mut handle::GrpcWebServerHandle) -> bool {
    std::panic::catch_unwind(|| {
        if handle.is_null() {
            return false;
        }

        let handle = unsafe { &*handle };
        if let Err(err) = handle.stop() {
            error!("grpc-web-server: failed to stop handle: {err}");
            return false;
        }

        true
    })
    .unwrap_or(false)
}

/// Destroys a server handle and releases all owned resources.
///
/// If a server thread is still active, this function requests graceful shutdown
/// and blocks until the thread exits before freeing the handle.
///
/// # Arguments
/// - `handle`: Pointer returned by `create`.
///
/// # Returns
/// - `true` when resources are released.
/// - `false` when `handle` is null.
///
/// # Safety
/// - `handle` must be a non-null owning pointer returned by `create`.
/// - `handle` must not have been passed to `destroy` before.
/// - No thread may be using `handle` when this function is called.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn destroy(handle: *mut handle::GrpcWebServerHandle) -> bool {
    std::panic::catch_unwind(|| {
        if handle.is_null() {
            return false;
        }

        let _ = unsafe { Box::from_raw(handle) };
        true
    })
    .unwrap_or(false)
}
