//! TLS certificate generation helpers and their C-compatible API.
//!
//! Rust callers can create certificate authorities with [`crate::tls::ca`] and
//! issue leaf certificates with [`crate::tls::leaf`]. C and C++ consumers use
//! [`crate::tls::ffi`], which keeps `rcgen` values behind opaque handles and
//! accepts subject alternative names as a C string array plus length.

/// Certificate authority creation and PEM output helpers.
pub mod ca;

/// C-compatible certificate authority and leaf certificate functions.
pub mod ffi;

/// Leaf certificate issuance and PEM output helpers.
pub mod leaf;
