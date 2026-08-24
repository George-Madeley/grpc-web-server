//! gRPC-Web reverse proxy server with native Rust and C-compatible APIs.
//!
//! The Rust server implementation is available through [`server::server`],
//! while its C/C++ lifecycle API is available through [`server::ffi`]. TLS
//! certificate-authority and leaf-certificate helpers are exposed through
//! [`tls::ca`] and [`tls::leaf`], with corresponding C/C++ exports in
//! [`tls::ffi`]. Each FFI function returning a non-null opaque handle transfers
//! ownership to the caller, which must release it with the documented destroy
//! function.

#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![warn(clippy::missing_docs_in_private_items)]
#![warn(clippy::missing_errors_doc)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::missing_safety_doc)]
#![warn(clippy::unnecessary_safety_doc)]

/// gRPC-Web server configuration, runtime, proxy, lifecycle handle, and C/C++ API.
pub mod server;

/// TLS certificate-authority and leaf-certificate helpers and C/C++ API.
pub mod tls;

mod utils;
