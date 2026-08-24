//! TLS certificate-authority and leaf-certificate FFI functions.

use std::{panic::AssertUnwindSafe, path::Path, slice};

use crate::utils::required_c_string;
use rcgen::{CertifiedIssuer, ExtendedKeyUsagePurpose, KeyPair};
use std::ffi::c_char;

use super::{ca, leaf};

/// Opaque certificate authority handle owned by C/C++ callers.
pub struct CertificateAuthorityHandle {
    authority: CertifiedIssuer<'static, KeyPair>,
}

/// Opaque leaf certificate handle owned by C/C++ callers.
pub struct LeafCertificateHandle {
    leaf: (rcgen::Certificate, KeyPair),
}

/// Extended key usage for a leaf certificate.
#[repr(C)]
pub enum CertificateUsage {
    /// Issue a certificate for TLS server authentication.
    ServerAuth,
    /// Issue a certificate for TLS client authentication.
    ClientAuth,
}

impl From<CertificateUsage> for ExtendedKeyUsagePurpose {
    fn from(usage: CertificateUsage) -> Self {
        match usage {
            CertificateUsage::ServerAuth => Self::ServerAuth,
            CertificateUsage::ClientAuth => Self::ClientAuth,
        }
    }
}

/// Converts a C array of strings into certificate subject alternative names.
///
/// # Safety
/// When `count` is non-zero, `values` must point to `count` valid pointers
/// to NUL-terminated UTF-8 C strings.
unsafe fn collect_subject_alt_names(
    values: *const *const c_char,
    count: usize,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if values.is_null() {
        return Err("subject_alt_names must be defined when count is non-zero".into());
    }

    unsafe { slice::from_raw_parts(values, count) }
        .iter()
        .map(|&value| unsafe { required_c_string(value) })
        .collect()
}

/// Creates a heap-allocated certificate authority.
///
/// The returned non-null pointer is owned by the caller and must be passed
/// exactly once to [`destroy_certificate_authority`].
///
/// # Safety
/// `common_name` must point to a valid NUL-terminated UTF-8 C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn create_certificate_authority(
    common_name: *const c_char,
) -> *mut CertificateAuthorityHandle {
    std::panic::catch_unwind(AssertUnwindSafe(|| {
        let common_name = match unsafe { required_c_string(common_name) } {
            Ok(common_name) => common_name,
            Err(_) => return std::ptr::null_mut(),
        };
        let authority = match ca::make(&common_name) {
            Ok(authority) => authority,
            Err(_) => return std::ptr::null_mut(),
        };
        Box::into_raw(Box::new(CertificateAuthorityHandle { authority }))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Writes a certificate authority to `ca.pem` and `ca-key.pem` in `directory`.
///
/// # Safety
/// `authority` must be a live pointer returned by
/// `create_certificate_authority`. `directory` must point to a valid
/// NUL-terminated UTF-8 C string. Neither may be destroyed concurrently.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn write_certificate_authority(
    authority: *const CertificateAuthorityHandle,
    directory: *const c_char,
) -> bool {
    std::panic::catch_unwind(AssertUnwindSafe(|| {
        let Some(authority) = (unsafe { authority.as_ref() }) else {
            return false;
        };
        let directory = match unsafe { required_c_string(directory) } {
            Ok(directory) => directory,
            Err(_) => return false,
        };
        ca::write(Path::new(&directory), &authority.authority).is_ok()
    }))
    .unwrap_or(false)
}

/// Issues a heap-allocated leaf certificate from a certificate authority.
///
/// The returned non-null pointer is owned by the caller and must be passed
/// exactly once to [`destroy_leaf_certificate`].
///
/// # Safety
/// - `authority` must be a live pointer returned by
///   `create_certificate_authority`.
/// - `common_name` must point to a valid NUL-terminated UTF-8 C string.
/// - If `subject_alt_name_count` is non-zero, `subject_alt_names` must point
///   to that many valid pointers to NUL-terminated UTF-8 C strings.
/// - `usage` must contain a valid [`CertificateUsage`] discriminant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn issue_leaf_certificate(
    authority: *const CertificateAuthorityHandle,
    common_name: *const c_char,
    subject_alt_names: *const *const c_char,
    subject_alt_name_count: usize,
    usage: CertificateUsage,
) -> *mut LeafCertificateHandle {
    std::panic::catch_unwind(AssertUnwindSafe(|| {
        let Some(authority) = (unsafe { authority.as_ref() }) else {
            return std::ptr::null_mut();
        };
        let common_name = match unsafe { required_c_string(common_name) } {
            Ok(common_name) => common_name,
            Err(_) => return std::ptr::null_mut(),
        };
        let subject_alt_names =
            match unsafe { collect_subject_alt_names(subject_alt_names, subject_alt_name_count) } {
                Ok(subject_alt_names) => subject_alt_names,
                Err(_) => return std::ptr::null_mut(),
            };
        let leaf = match leaf::issue(
            &authority.authority,
            &common_name,
            subject_alt_names,
            usage.into(),
        ) {
            Ok(leaf) => leaf,
            Err(_) => return std::ptr::null_mut(),
        };
        Box::into_raw(Box::new(LeafCertificateHandle { leaf }))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Writes a leaf certificate to `{prefix}.pem` and `{prefix}-key.pem`.
///
/// # Safety
/// `leaf` must be a live pointer returned by `issue_leaf_certificate`.
/// `directory` and `prefix` must point to valid NUL-terminated UTF-8 C
/// strings. The leaf must not be destroyed concurrently.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn write_leaf_certificate(
    leaf: *const LeafCertificateHandle,
    directory: *const c_char,
    prefix: *const c_char,
) -> bool {
    std::panic::catch_unwind(AssertUnwindSafe(|| {
        let Some(leaf) = (unsafe { leaf.as_ref() }) else {
            return false;
        };
        let directory = match unsafe { required_c_string(directory) } {
            Ok(directory) => directory,
            Err(_) => return false,
        };
        let prefix = match unsafe { required_c_string(prefix) } {
            Ok(prefix) => prefix,
            Err(_) => return false,
        };
        leaf::write(Path::new(&directory), &prefix, &leaf.leaf).is_ok()
    }))
    .unwrap_or(false)
}

/// Destroys a certificate authority handle and releases its resources.
///
/// # Safety
/// `authority` must be a non-null pointer returned by
/// `create_certificate_authority`, must not have been destroyed before,
/// and must not be in use by another thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn destroy_certificate_authority(
    authority: *mut CertificateAuthorityHandle,
) -> bool {
    std::panic::catch_unwind(AssertUnwindSafe(|| {
        if authority.is_null() {
            return false;
        }
        let _ = unsafe { Box::from_raw(authority) };
        true
    }))
    .unwrap_or(false)
}

/// Destroys a leaf certificate handle and releases its resources.
///
/// # Safety
/// `leaf` must be a non-null pointer returned by `issue_leaf_certificate`,
/// must not have been destroyed before, and must not be in use by another
/// thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn destroy_leaf_certificate(leaf: *mut LeafCertificateHandle) -> bool {
    std::panic::catch_unwind(AssertUnwindSafe(|| {
        if leaf.is_null() {
            return false;
        }
        let _ = unsafe { Box::from_raw(leaf) };
        true
    }))
    .unwrap_or(false)
}
