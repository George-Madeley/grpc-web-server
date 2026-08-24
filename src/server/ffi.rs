//! gRPC-Web server configuration and lifecycle functions.

use std::{path::PathBuf, sync::Once};

use tracing::error;
use tracing_subscriber::{EnvFilter, fmt};

use std::ffi::c_char;

use super::{handle, server as server_impl};
use crate::utils::{collect_c_string_vector, required_c_string};

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

/// C-compatible CORS settings for one gRPC-Web server instance.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct GrpcWebCorsOptions {
    /// Pointer to an array of allowed origin strings.
    pub allowed_origins: *const *const c_char,
    /// Number of entries in `allowed_origins`.
    pub allowed_origins_count: usize,
    /// Whether to allow any origin (`*`).
    pub allow_any_origin: bool,
    /// Pointer to an array of allowed HTTP method strings.
    pub allowed_methods: *const *const c_char,
    /// Number of entries in `allowed_methods`.
    pub allowed_methods_count: usize,
    /// Whether to allow any request method.
    pub allow_any_method: bool,
    /// Pointer to an array of allowed request header names.
    pub allowed_headers: *const *const c_char,
    /// Number of entries in `allowed_headers`.
    pub allowed_headers_count: usize,
    /// Whether to allow any request header.
    pub allow_any_header: bool,
    /// Whether to send `Access-Control-Allow-Credentials: true`.
    pub allow_credentials: bool,
}

impl TryInto<server_impl::CorsPolicy> for GrpcWebCorsOptions {
    type Error = Box<dyn std::error::Error>;

    fn try_into(self) -> Result<server_impl::CorsPolicy, Self::Error> {
        let origin_values =
            unsafe { collect_c_string_vector(self.allowed_origins, self.allowed_origins_count)? };
        let allowed_origins = origin_values
            .iter()
            .map(|val| {
                http::HeaderValue::from_str(val.as_str())
                    .map_err(|err| format!("invalid CORS origin '{val}': {err}"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let method_values =
            unsafe { collect_c_string_vector(self.allowed_methods, self.allowed_methods_count)? };
        let allowed_methods = method_values
            .iter()
            .map(|val| {
                http::Method::from_bytes(val.as_bytes())
                    .map_err(|err| format!("invalid CORS method '{val}': {err}"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let header_values =
            unsafe { collect_c_string_vector(self.allowed_headers, self.allowed_headers_count)? };
        let allowed_headers = header_values
            .iter()
            .map(|val| {
                http::HeaderName::from_bytes(val.as_bytes())
                    .map_err(|err| format!("invalid CORS header '{val}': {err}"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(server_impl::CorsPolicy {
            allowed_origins: if self.allow_any_origin {
                None
            } else if allowed_origins.is_empty() {
                Some(Vec::new())
            } else {
                Some(allowed_origins)
            },
            allow_any_origin: self.allow_any_origin,
            allowed_methods: if self.allow_any_method {
                None
            } else if allowed_methods.is_empty() {
                Some(Vec::new())
            } else {
                Some(allowed_methods)
            },
            allow_any_method: self.allow_any_method,
            allowed_headers: if self.allow_any_header {
                None
            } else if allowed_headers.is_empty() {
                Some(Vec::new())
            } else {
                Some(allowed_headers)
            },
            allow_any_header: self.allow_any_header,
            allow_credentials: self.allow_credentials,
        })
    }
}

/// C-compatible options used to configure one gRPC-Web server handle.
///
/// Each non-null string pointer must refer to a valid, NUL-terminated UTF-8
/// string for the duration of [`create`]. `create` copies each string, so
/// the caller may release the original strings after it returns.
/// `http_address` and `grpc_address` are required; all other string fields
/// are optional.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct GrpcWebServerOptions {
    /// Address on which to host the HTTP/1.1 proxy and web server.
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
    /// Browser-facing CORS settings for this server instance.
    pub cors_options: GrpcWebCorsOptions,
}

impl TryInto<server_impl::GrpcWebServerOptions> for GrpcWebServerOptions {
    type Error = Box<dyn std::error::Error>;

    fn try_into(self) -> Result<server_impl::GrpcWebServerOptions, Self::Error> {
        let optional_string = |ptr: *const c_char| {
            if ptr.is_null() {
                Ok(None)
            } else {
                unsafe { required_c_string(ptr).map(Some) }
            }
        };

        Ok(server_impl::GrpcWebServerOptions {
            http_address: unsafe { required_c_string(self.http_address) }?,
            grpc_address: unsafe { required_c_string(self.grpc_address) }?,
            static_dir: optional_string(self.static_dir)?,
            grpc_ca_cert: optional_string(self.grpc_ca_cert)?.map(PathBuf::from),
            grpc_proxy_key: optional_string(self.grpc_proxy_key)?.map(PathBuf::from),
            grpc_proxy_cert: optional_string(self.grpc_proxy_cert)?.map(PathBuf::from),
            http_key: optional_string(self.http_key)?.map(PathBuf::from),
            http_cert: optional_string(self.http_cert)?.map(PathBuf::from),
            cors_policy: self.cors_options.try_into()?,
        })
    }
}

/// Process-wide one-time tracing initialization for all server handles.
static TRACING_INIT: Once = Once::new();

/// Initializes tracing once using the level from the first created handle.
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
/// The returned non-null pointer transfers ownership of the allocation to
/// the caller. Pass it to `start`, `stop`, `wait`, and `is_running`, then
/// pass it exactly once to `destroy` to release it. This function configures
/// but does not start the server.
///
/// # Returns
/// A non-null owning handle on success, or null after validation, allocation,
/// or panic failure.
///
/// # Safety
/// - Each non-null string pointer in `options` must be valid,
///   NUL-terminated, and UTF-8.
/// - `options.log_level` must contain a valid [`LogLevel`] discriminant.
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

        Box::into_raw(Box::new(handle::GrpcWebServerHandle::new(server_options)))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Starts a server handle in a background thread.
///
/// # Returns
/// `true` when a new background runtime starts; otherwise `false`.
///
/// # Safety
/// `handle` must be a live pointer returned by `create`, and no thread may
/// call `destroy` while this function uses it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn start(handle: *mut handle::GrpcWebServerHandle) -> bool {
    std::panic::catch_unwind(|| {
        let Some(handle) = (unsafe { handle.as_ref() }) else {
            return false;
        };

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
/// # Returns
/// `true` when the call completes successfully; `false` for a null handle
/// or lifecycle error.
///
/// # Safety
/// `handle` must be a live pointer returned by `create`, and no thread may
/// call `destroy` while this function uses it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wait(handle: *mut handle::GrpcWebServerHandle) -> bool {
    std::panic::catch_unwind(|| {
        let Some(handle) = (unsafe { handle.as_ref() }) else {
            return false;
        };

        if let Err(err) = handle.wait() {
            error!("grpc-web-server: failed to wait for handle: {err}");
            return false;
        }
        true
    })
    .unwrap_or(false)
}

/// Returns whether `handle` is currently marked as running.
///
/// # Safety
/// `handle` must be a live pointer returned by `create`, and no thread may
/// call `destroy` while this function uses it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn is_running(handle: *const handle::GrpcWebServerHandle) -> bool {
    let Some(handle) = (unsafe { handle.as_ref() }) else {
        return false;
    };
    handle.is_running()
}

/// Requests graceful shutdown for the running server, if any.
///
/// This function does not wait for shutdown to finish; use [`wait`] when
/// the caller needs to wait for the background thread to exit.
///
/// # Returns
/// `true` if the request completes; `false` for a null handle or lifecycle
/// error.
///
/// # Safety
/// `handle` must be a live pointer returned by `create`, and no thread may
/// call `destroy` while this function uses it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stop(handle: *mut handle::GrpcWebServerHandle) -> bool {
    std::panic::catch_unwind(|| {
        let Some(handle) = (unsafe { handle.as_ref() }) else {
            return false;
        };

        if let Err(err) = handle.stop() {
            error!("grpc-web-server: failed to stop handle: {err}");
            return false;
        }
        true
    })
    .unwrap_or(false)
}

/// Destroys a server handle and releases all of its resources.
///
/// If a server thread is active, dropping the handle requests graceful
/// shutdown and waits for that thread before freeing the allocation.
///
/// # Safety
/// `handle` must be a non-null pointer returned by `create`, must not have
/// been passed to `destroy` before, and must not be used by another thread.
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
