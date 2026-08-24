//! gRPC-Web reverse proxy server with native Rust and C-compatible APIs.
//!
//! Use [`server::GrpcWebServer`] to construct and run a server from Rust, or
//! use [`ffi`] to create opaque handles from C or C++. Each handle represents
//! an independently configured server instance.

#![warn(missing_docs)]
#![warn(clippy::missing_docs_in_private_items)]
#![warn(clippy::missing_errors_doc)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::missing_safety_doc)]
#![warn(clippy::unnecessary_safety_doc)]

/// gRPC-Web request forwarding service.
pub mod proxy;

/// Server configuration and runtime implementation.
pub mod server;

/// Certificate-authority and leaf-certificate generation helpers.
pub mod tls;

/// C-compatible API for managing opaque server handles.
pub mod ffi;

/// Per-instance server lifecycle handle implementation.
pub mod handle;
