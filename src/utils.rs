use std::{
    ffi::{CStr, c_char},
    slice,
};

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

/// Converts a C array of strings into an owned Rust string vector.
///
/// # Safety
/// When `count` is non-zero, `values` must point to `count` valid pointers
/// to NUL-terminated UTF-8 C strings.
pub unsafe fn collect_c_string_vector(
    values: *const *const c_char,
    count: usize,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if values.is_null() {
        return Err("values must be defined when count is non-zero".into());
    }

    unsafe { slice::from_raw_parts(values, count) }
        .iter()
        .map(|&value| unsafe { required_c_string(value) })
        .collect()
}
