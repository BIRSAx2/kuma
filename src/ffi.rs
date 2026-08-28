//! C interface for Kuma.
//!
//! Unsafe pointer conversion is confined to this module.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

fn default_target() -> &'static crate::ir::Target {
    #[cfg(target_os = "macos")]
    {
        #[cfg(target_arch = "x86_64")]
        {
            &crate::amd64::T_AMD64_APPLE
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            &crate::arm64::T_ARM64_APPLE
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        &crate::amd64::T_AMD64_SYSV
    }
}


/// Convert a caller-owned byte range to UTF-8.
///
/// # Safety
/// The caller must ensure `input` points to `input_len` valid UTF-8 bytes.
unsafe fn slice_from_raw(input: *const u8, input_len: c_int) -> &'static str {
    let bytes = std::slice::from_raw_parts(input, input_len as usize);
    std::str::from_utf8_unchecked(bytes)
}

/// Copy a caller-owned C string.
///
/// # Safety
/// The pointer must be non-null and point to a valid C string.
unsafe fn string_from_cstr(p: *const c_char) -> String {
    CStr::from_ptr(p).to_string_lossy().into_owned()
}

/// Write assembly to a unique temporary file.
fn write_temp_asm(asm: &str) -> std::io::Result<std::path::PathBuf> {
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("kuma_{pid}_{ts}.s"));
    std::fs::write(&path, asm)?;
    Ok(path)
}

/// Return the macOS SDK path reported by `xcrun`.
fn sdk_path() -> Option<String> {
    let out = std::process::Command::new("xcrun")
        .args(["--show-sdk-path"])
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}


/// Compile IR to host-target assembly.
///
/// Returns a heap-allocated C string (caller must free with `kuma_free`),
/// or `NULL` on error.
#[no_mangle]
pub extern "C" fn kuma_compile(input: *const u8, input_len: c_int) -> *mut c_char {
    if input.is_null() || input_len <= 0 {
        return ptr::null_mut();
    }

    let src = unsafe { slice_from_raw(input, input_len) };

    match crate::compile(src, default_target()) {
        Ok(asm) => match CString::new(asm) {
            Ok(c) => c.into_raw(),
            Err(_) => ptr::null_mut(), // interior NUL: shouldn't happen
        },
        Err(e) => {
            eprintln!("kuma error: {e}");
            ptr::null_mut()
        }
    }
}

/// Free a string previously returned by `kuma_compile`.
#[no_mangle]
pub extern "C" fn kuma_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            drop(CString::from_raw(ptr));
        }
    }
}

/// Compile IR to `output_obj`.
///
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn kuma_assemble(
    input: *const u8,
    input_len: c_int,
    output_obj: *const c_char,
) -> c_int {
    if input.is_null() || input_len <= 0 || output_obj.is_null() {
        return -1;
    }

    let src = unsafe { slice_from_raw(input, input_len) };
    let obj_path = unsafe { string_from_cstr(output_obj) };

    let asm = match crate::compile(src, default_target()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("kuma compile error: {e}");
            return -1;
        }
    };

    let asm_path = match write_temp_asm(&asm) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("kuma: failed to write temp asm: {e}");
            return -1;
        }
    };

    let status = std::process::Command::new("/usr/bin/as")
        .args(["-o", &obj_path, asm_path.to_str().unwrap_or("")])
        .status();

    let _ = std::fs::remove_file(&asm_path);

    match status {
        Ok(s) if s.success() => 0,
        Ok(s) => {
            eprintln!(
                "kuma: assembler failed with exit code {}",
                s.code().unwrap_or(-1)
            );
            -1
        }
        Err(e) => {
            eprintln!("kuma: failed to run assembler: {e}");
            -1
        }
    }
}

/// Compile IR and link it with any extra object files.
///
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn kuma_compile_and_link(
    input: *const u8,
    input_len: c_int,
    output_path: *const c_char,
    extra_objects: *const *const c_char,
    num_extra: c_int,
) -> c_int {
    if input.is_null() || input_len <= 0 || output_path.is_null() {
        return -1;
    }

    let src = unsafe { slice_from_raw(input, input_len) };
    let out_path = unsafe { string_from_cstr(output_path) };

    let asm = match crate::compile(src, default_target()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("kuma compile error: {e}");
            return -1;
        }
    };

    let asm_path = match write_temp_asm(&asm) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("kuma: failed to write temp asm: {e}");
            return -1;
        }
    };

    let obj_path = asm_path.with_extension("o");
    let as_status = std::process::Command::new("/usr/bin/as")
        .args([
            "-o",
            obj_path.to_str().unwrap_or(""),
            asm_path.to_str().unwrap_or(""),
        ])
        .status();

    let _ = std::fs::remove_file(&asm_path);

    match as_status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!(
                "kuma: assembler failed with exit code {}",
                s.code().unwrap_or(-1)
            );
            return -1;
        }
        Err(e) => {
            eprintln!("kuma: failed to run assembler: {e}");
            return -1;
        }
    }

    let mut args: Vec<String> = vec![
        "-o".to_string(),
        out_path.clone(),
        obj_path.to_str().unwrap_or("").to_string(),
    ];

    if !extra_objects.is_null() && num_extra > 0 {
        for i in 0..num_extra as usize {
            let obj_ptr = unsafe { *extra_objects.add(i) };
            if !obj_ptr.is_null() {
                let obj = unsafe { string_from_cstr(obj_ptr) };
                args.push(obj);
            }
        }
    }

    if let Some(sdk) = sdk_path() {
        args.push("-isysroot".to_string());
        args.push(sdk);
    }

    args.push("-lSystem".to_string());
    args.push("-lm".to_string());

    let link_status = std::process::Command::new("cc").args(&args).status();

    let _ = std::fs::remove_file(&obj_path);

    match link_status {
        Ok(s) if s.success() => 0,
        Ok(s) => {
            eprintln!(
                "kuma: linker failed with exit code {}",
                s.code().unwrap_or(-1)
            );
            -1
        }
        Err(e) => {
            eprintln!("kuma: failed to run linker: {e}");
            -1
        }
    }
}
