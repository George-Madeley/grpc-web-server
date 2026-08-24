//! gRPC-Web server implementation and its C-compatible lifecycle API.
//!
//! Use [`crate::server::server::GrpcWebServer`] and
//! [`crate::server::server::GrpcWebServerOptions`] from Rust. C and C++
//! consumers use [`crate::server::ffi`] to create an opaque
//! [`crate::server::handle::GrpcWebServerHandle`], control its lifecycle, and
//! destroy it when no longer needed.

/// C-compatible server configuration and lifecycle functions.
pub mod ffi;

/// Per-instance server lifecycle handle implementation.
pub mod handle;

/// gRPC-Web request forwarding service.
pub mod proxy;

/// Server configuration and runtime implementation.
pub mod server;
