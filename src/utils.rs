use std::ffi::{CStr, c_char};

/// Converts a non-null C string pointer into an owned Rust string.
///
/// # Safety
/// `ptr` must point to a valid NUL-terminated UTF-8 C string.
pub unsafe fn required_c_string(ptr: *const c_char) -> Result<String, Box<dyn std::error::Error>> {
    if ptr.is_null() {
        return Err("string must be defined".into());
    }

    let c_str = unsafe { CStr::from_ptr(ptr) };
    Ok(c_str.to_str()?.to_owned())
}
