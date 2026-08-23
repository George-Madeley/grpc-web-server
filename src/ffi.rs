use std::{
    ffi::{CStr, c_char},
    path::PathBuf,
    sync::Once,
};

use tracing::error;
use tracing_subscriber::{EnvFilter, fmt};

use crate::{handle::GrpcWebServerHandle, server::ServerOptions};

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

/// Process-wide one-time tracing initialization for all server handles.
static TRACING_INIT: Once = Once::new();

/// Initializes tracing once for the process.
///
/// The first created handle decides the initial log level. Later calls keep the
/// existing subscriber and ignore new levels.
fn init_tracing_once(level: LogLevel) {
    TRACING_INIT.call_once(|| {
        let filter = EnvFilter::new(format!("grpc_web_server={},tower_http=info", level.as_str()));
        let _ = fmt().with_env_filter(filter).try_init();
    });
}

/// Creates a new server handle from FFI options.
///
/// # Arguments
/// - `options`: FFI server options passed from C/C++.
///
/// # Returns
/// - Non-null pointer to a newly allocated server handle on success.
/// - Null pointer on option validation or allocation failure.
///
/// # Safety
/// - Any non-null C string pointer in `options` must be valid and NUL-terminated.
///
/// # Panics
/// - This function catches unwinds to avoid unwinding across the FFI boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn create(options: GrpcWebProxyOptions) -> *mut GrpcWebServerHandle {
    std::panic::catch_unwind(|| {
        init_tracing_once(options.log_level.clone());

        let server_options = match options.try_into() {
            Ok(options) => options,
            Err(err) => {
                error!("grpc-web-server: invalid options: {err}");
                return std::ptr::null_mut();
            }
        };

        let handle = GrpcWebServerHandle::new(server_options);
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
/// - `handle` must point to a valid handle created by `create`.
///
/// # Panics
/// - This function catches unwinds to avoid unwinding across the FFI boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn start(handle: *mut GrpcWebServerHandle) -> bool {
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
/// - `handle` must point to a valid handle created by `create`.
///
/// # Panics
/// - This function catches unwinds to avoid unwinding across the FFI boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wait(handle: *mut GrpcWebServerHandle) -> bool {
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
/// - `handle` must point to a valid handle created by `create`.
///
/// # Panics
/// - This function does not intentionally panic.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn is_running(handle: *const GrpcWebServerHandle) -> bool {
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
/// - `handle` must point to a valid handle created by `create`.
///
/// # Panics
/// - This function catches unwinds to avoid unwinding across the FFI boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stop(handle: *mut GrpcWebServerHandle) -> bool {
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
/// - `handle` must be a pointer returned by `create`.
/// - `handle` must be destroyed at most once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn destroy(handle: *mut GrpcWebServerHandle) -> bool {
    std::panic::catch_unwind(|| {
        if handle.is_null() {
            return false;
        }

        let _ = unsafe { Box::from_raw(handle) };
        true
    })
    .unwrap_or(false)
}
