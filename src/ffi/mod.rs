//! Optional C interface.
//!
//! # Safety invariants
//!
//! All pointer interpretation and owned-buffer reconstruction is confined to
//! this module. Callers must provide readable ranges for `(input, length)`,
//! valid NUL-terminated path strings, and writable `KumaBuffer` pointers.
//! Buffers must be released exactly once with [`kuma_buffer_free`]. Every
//! exported operation catches panics before they can cross the C seam.

use std::ffi::{CStr, OsStr, OsString};
use std::fmt;
use std::io::Write;
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{CompileError, Target};

const TARGET_AMD64_SYSV: u32 = 1;
const TARGET_AMD64_APPLE: u32 = 2;
const TARGET_AARCH64_ELF: u32 = 3;
const TARGET_AARCH64_APPLE: u32 = 4;

/// Stable status values returned by every fallible C operation.
#[repr(u32)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum KumaStatus {
    Success = 0,
    InvalidArgument = 1,
    InvalidUtf8 = 2,
    ParseError = 3,
    InvalidIr = 4,
    UnsupportedTarget = 5,
    IoError = 6,
    ToolchainError = 7,
    InternalError = 8,
}

/// An owned byte buffer allocated by Kuma.
#[repr(C)]
#[derive(Debug)]
pub struct KumaBuffer {
    pub data: *mut u8,
    pub length: usize,
    pub capacity: usize,
}

impl KumaBuffer {
    const fn empty() -> Self {
        Self {
            data: std::ptr::null_mut(),
            length: 0,
            capacity: 0,
        }
    }

    fn from_string(value: String) -> Self {
        let mut bytes = value.into_bytes();
        if bytes.is_empty() {
            return Self::empty();
        }
        let buffer = Self {
            data: bytes.as_mut_ptr(),
            length: bytes.len(),
            capacity: bytes.capacity(),
        };
        std::mem::forget(bytes);
        buffer
    }
}

#[derive(Debug)]
struct Failure {
    status: KumaStatus,
    message: String,
}

impl Failure {
    fn new(status: KumaStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}

fn target_from_raw(raw: u32) -> Result<Target, Failure> {
    match raw {
        TARGET_AMD64_SYSV => Ok(Target::Amd64SysV),
        TARGET_AMD64_APPLE => Ok(Target::Amd64Apple),
        TARGET_AARCH64_ELF => Ok(Target::Aarch64Elf),
        TARGET_AARCH64_APPLE => Ok(Target::Aarch64Apple),
        _ => Err(Failure::new(
            KumaStatus::UnsupportedTarget,
            format!("unsupported Kuma target value {raw}"),
        )),
    }
}

fn host_target() -> Option<Target> {
    if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some(Target::Amd64Apple)
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some(Target::Aarch64Apple)
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some(Target::Amd64SysV)
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some(Target::Aarch64Elf)
    } else {
        None
    }
}

fn require_host_target(target: Target) -> Result<(), Failure> {
    if host_target() == Some(target) {
        Ok(())
    } else {
        Err(Failure::new(
            KumaStatus::UnsupportedTarget,
            "assembly and linking require the target matching this host",
        ))
    }
}

fn compile_failure(error: CompileError) -> Failure {
    let status = match error {
        CompileError::Parse(_) => KumaStatus::ParseError,
        CompileError::InvalidIr(_) => KumaStatus::InvalidIr,
        CompileError::Internal(_) => KumaStatus::InternalError,
    };
    Failure::new(status, error.to_string())
}

unsafe fn initialize_buffer(buffer: *mut KumaBuffer) -> Result<(), Failure> {
    if buffer.is_null() {
        return Err(Failure::new(
            KumaStatus::InvalidArgument,
            "a required output buffer pointer was null",
        ));
    }
    // SAFETY: the caller promises a writable KumaBuffer pointer.
    unsafe { buffer.write(KumaBuffer::empty()) };
    Ok(())
}

unsafe fn set_buffer(buffer: *mut KumaBuffer, value: String) {
    // SAFETY: operation entry points validate and initialize this pointer.
    unsafe { buffer.write(KumaBuffer::from_string(value)) };
}

unsafe fn source_from_raw<'a>(input: *const u8, length: usize) -> Result<&'a str, Failure> {
    if input.is_null() {
        return Err(Failure::new(
            KumaStatus::InvalidArgument,
            "input pointer was null",
        ));
    }
    // SAFETY: the caller promises `length` readable bytes at `input`.
    let bytes = unsafe { std::slice::from_raw_parts(input, length) };
    std::str::from_utf8(bytes).map_err(|error| {
        Failure::new(
            KumaStatus::InvalidUtf8,
            format!("input was not valid UTF-8: {error}"),
        )
    })
}

unsafe fn path_from_raw<'a>(path: *const c_char, label: &str) -> Result<&'a Path, Failure> {
    if path.is_null() {
        return Err(Failure::new(
            KumaStatus::InvalidArgument,
            format!("{label} pointer was null"),
        ));
    }
    // SAFETY: the caller promises `path` points to a NUL-terminated C string.
    let value = unsafe { CStr::from_ptr(path) }.to_str().map_err(|error| {
        Failure::new(
            KumaStatus::InvalidUtf8,
            format!("{label} was not valid UTF-8: {error}"),
        )
    })?;
    if value.is_empty() {
        return Err(Failure::new(
            KumaStatus::InvalidArgument,
            format!("{label} was empty"),
        ));
    }
    Ok(Path::new(value))
}

fn compile_source(source: &str, raw_target: u32) -> Result<(String, Target), Failure> {
    let target = target_from_raw(raw_target)?;
    crate::compile(source, target)
        .map(|assembly| (assembly, target))
        .map_err(compile_failure)
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else {
        "unknown panic payload".to_owned()
    }
}

unsafe fn finish(result: Result<(), Failure>, error_buffer: *mut KumaBuffer) -> KumaStatus {
    match result {
        Ok(()) => KumaStatus::Success,
        Err(failure) => {
            if !error_buffer.is_null() {
                // SAFETY: entry points initialize a non-null error buffer.
                unsafe { set_buffer(error_buffer, failure.message) };
            }
            failure.status
        }
    }
}

unsafe fn catch_operation(
    error_buffer: *mut KumaBuffer,
    operation: impl FnOnce() -> Result<(), Failure>,
) -> KumaStatus {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(result) => {
            // SAFETY: propagated from the exported operation's pointer contract.
            unsafe { finish(result, error_buffer) }
        }
        Err(payload) => {
            if !error_buffer.is_null() {
                // SAFETY: exported operations initialize a non-null error buffer
                // before doing work that can panic.
                unsafe {
                    error_buffer.write(KumaBuffer::empty());
                    set_buffer(
                        error_buffer,
                        format!(
                            "internal compiler error: {}",
                            panic_message(payload.as_ref())
                        ),
                    );
                }
            }
            KumaStatus::InternalError
        }
    }
}

/// Compile textual IR to target assembly.
#[unsafe(no_mangle)]
pub extern "C" fn kuma_compile(
    input: *const u8,
    input_length: usize,
    target: u32,
    assembly: *mut KumaBuffer,
    error: *mut KumaBuffer,
) -> KumaStatus {
    // SAFETY: all pointer operations are guarded by the documented C contract.
    unsafe {
        catch_operation(error, || {
            if assembly == error {
                initialize_buffer(error)?;
                return Err(Failure::new(
                    KumaStatus::InvalidArgument,
                    "assembly and error buffers must be distinct",
                ));
            }
            initialize_buffer(assembly)?;
            initialize_buffer(error)?;
            let source = source_from_raw(input, input_length)?;
            let (output, _) = compile_source(source, target)?;
            set_buffer(assembly, output);
            Ok(())
        })
    }
}

#[derive(Debug)]
struct ToolOutput {
    success: bool,
    code: Option<i32>,
    stderr: String,
}

trait ProcessRunner {
    fn run(&self, program: &OsStr, arguments: &[OsString]) -> std::io::Result<ToolOutput>;
}

struct SystemProcessRunner;

impl ProcessRunner for SystemProcessRunner {
    fn run(&self, program: &OsStr, arguments: &[OsString]) -> std::io::Result<ToolOutput> {
        let output = Command::new(program).args(arguments).output()?;
        Ok(ToolOutput {
            success: output.status.success(),
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TemporaryAssembly {
    path: PathBuf,
}

impl TemporaryAssembly {
    fn create(assembly: &str) -> Result<Self, Failure> {
        for _ in 0..128 {
            let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("kuma-{}-{count}.s", std::process::id()));
            let mut file = match std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(Failure::new(
                        KumaStatus::IoError,
                        format!("failed to create temporary assembly: {error}"),
                    ));
                }
            };
            let temporary = Self { path };
            file.write_all(assembly.as_bytes()).map_err(|error| {
                Failure::new(
                    KumaStatus::IoError,
                    format!("failed to write temporary assembly: {error}"),
                )
            })?;
            return Ok(temporary);
        }
        Err(Failure::new(
            KumaStatus::IoError,
            "could not reserve a unique temporary assembly path",
        ))
    }
}

impl Drop for TemporaryAssembly {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn compiler_driver() -> OsString {
    std::env::var_os("CC").unwrap_or_else(|| OsString::from("cc"))
}

fn run_tool(
    runner: &dyn ProcessRunner,
    program: &OsStr,
    arguments: &[OsString],
    action: &str,
) -> Result<(), Failure> {
    let output = runner.run(program, arguments).map_err(|error| {
        Failure::new(
            KumaStatus::ToolchainError,
            format!("failed to run compiler driver for {action}: {error}"),
        )
    })?;
    if output.success {
        return Ok(());
    }
    let code = output
        .code
        .map_or_else(|| "signal".to_owned(), |code| code.to_string());
    let detail = output.stderr.trim();
    Err(Failure::new(
        KumaStatus::ToolchainError,
        if detail.is_empty() {
            format!("compiler driver failed during {action} ({code})")
        } else {
            format!("compiler driver failed during {action} ({code}): {detail}")
        },
    ))
}

fn assemble_with(runner: &dyn ProcessRunner, assembly: &str, output: &Path) -> Result<(), Failure> {
    let temporary = TemporaryAssembly::create(assembly)?;
    let arguments = [
        OsString::from("-c"),
        temporary.path.as_os_str().to_owned(),
        OsString::from("-o"),
        output.as_os_str().to_owned(),
    ];
    run_tool(runner, &compiler_driver(), &arguments, "assembly")
}

fn link_with(
    runner: &dyn ProcessRunner,
    assembly: &str,
    output: &Path,
    extra_objects: &[PathBuf],
) -> Result<(), Failure> {
    let temporary = TemporaryAssembly::create(assembly)?;
    let mut arguments = Vec::with_capacity(extra_objects.len() + 4);
    arguments.push(temporary.path.as_os_str().to_owned());
    arguments.extend(extra_objects.iter().map(|path| path.as_os_str().to_owned()));
    arguments.push(OsString::from("-o"));
    arguments.push(output.as_os_str().to_owned());
    arguments.push(OsString::from("-lm"));
    run_tool(runner, &compiler_driver(), &arguments, "linking")
}

/// Compile textual IR and assemble it to a host object file.
#[unsafe(no_mangle)]
pub extern "C" fn kuma_assemble(
    input: *const u8,
    input_length: usize,
    target: u32,
    output_object: *const c_char,
    error: *mut KumaBuffer,
) -> KumaStatus {
    // SAFETY: all pointer operations are guarded by the documented C contract.
    unsafe {
        catch_operation(error, || {
            initialize_buffer(error)?;
            let source = source_from_raw(input, input_length)?;
            let output = path_from_raw(output_object, "output object path")?;
            let (assembly, target) = compile_source(source, target)?;
            require_host_target(target)?;
            assemble_with(&SystemProcessRunner, &assembly, output)
        })
    }
}

unsafe fn extra_objects_from_raw(
    objects: *const *const c_char,
    count: usize,
) -> Result<Vec<PathBuf>, Failure> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if objects.is_null() {
        return Err(Failure::new(
            KumaStatus::InvalidArgument,
            "extra object array was null with a non-zero count",
        ));
    }
    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        // SAFETY: the caller promises an array with `count` readable pointers.
        let object = unsafe { *objects.add(index) };
        // SAFETY: every array entry must be a valid C path string.
        result.push(unsafe { path_from_raw(object, "extra object path")? }.to_owned());
    }
    Ok(result)
}

/// Compile textual IR and link it with optional extra objects on the host.
#[unsafe(no_mangle)]
pub extern "C" fn kuma_compile_and_link(
    input: *const u8,
    input_length: usize,
    target: u32,
    output_path: *const c_char,
    extra_objects: *const *const c_char,
    extra_object_count: usize,
    error: *mut KumaBuffer,
) -> KumaStatus {
    // SAFETY: all pointer operations are guarded by the documented C contract.
    unsafe {
        catch_operation(error, || {
            initialize_buffer(error)?;
            let source = source_from_raw(input, input_length)?;
            let output = path_from_raw(output_path, "output path")?;
            let extras = extra_objects_from_raw(extra_objects, extra_object_count)?;
            let (assembly, target) = compile_source(source, target)?;
            require_host_target(target)?;
            link_with(&SystemProcessRunner, &assembly, output, &extras)
        })
    }
}

/// Release a buffer returned by Kuma and reset it to the empty state.
#[unsafe(no_mangle)]
pub extern "C" fn kuma_buffer_free(buffer: *mut KumaBuffer) {
    if buffer.is_null() {
        return;
    }
    // SAFETY: callers may only pass a buffer initialized by Kuma. Replacing it
    // first makes an accidental second call harmless.
    let owned = unsafe { std::ptr::replace(buffer, KumaBuffer::empty()) };
    if !owned.data.is_null() && owned.capacity != 0 {
        // SAFETY: `from_string` produced exactly this pointer/length/capacity.
        unsafe {
            drop(Vec::from_raw_parts(
                owned.data,
                owned.length,
                owned.capacity,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<(OsString, Vec<OsString>)>>,
    }

    impl ProcessRunner for FakeRunner {
        fn run(&self, program: &OsStr, arguments: &[OsString]) -> std::io::Result<ToolOutput> {
            self.calls
                .lock()
                .expect("fake runner lock poisoned")
                .push((program.to_owned(), arguments.to_vec()));
            Ok(ToolOutput {
                success: true,
                code: Some(0),
                stderr: String::new(),
            })
        }
    }

    struct FailedRunner;

    impl ProcessRunner for FailedRunner {
        fn run(&self, _program: &OsStr, _arguments: &[OsString]) -> std::io::Result<ToolOutput> {
            Ok(ToolOutput {
                success: false,
                code: Some(2),
                stderr: "synthetic toolchain failure".to_owned(),
            })
        }
    }

    #[test]
    fn assemble_uses_compiler_driver() {
        let runner = FakeRunner::default();
        assemble_with(&runner, ".text\n", Path::new("out.o")).expect("assembly command");
        let calls = runner.calls.lock().expect("fake runner lock poisoned");
        assert_eq!(calls.len(), 1);
        assert!(calls[0].1.iter().any(|argument| argument == "-c"));
        assert!(calls[0].1.iter().any(|argument| argument == "out.o"));
    }

    #[test]
    fn link_preserves_extra_objects() {
        let runner = FakeRunner::default();
        link_with(
            &runner,
            ".text\n",
            Path::new("program"),
            &[PathBuf::from("one.o"), PathBuf::from("two.o")],
        )
        .expect("link command");
        let calls = runner.calls.lock().expect("fake runner lock poisoned");
        assert!(calls[0].1.iter().any(|argument| argument == "one.o"));
        assert!(calls[0].1.iter().any(|argument| argument == "two.o"));
        assert!(!calls[0].1.iter().any(|argument| argument == "-lSystem"));
    }

    #[test]
    fn toolchain_failure_preserves_status_and_stderr() {
        let failure = run_tool(&FailedRunner, OsStr::new("cc"), &[], "testing")
            .expect_err("failed process accepted");
        assert_eq!(failure.status, KumaStatus::ToolchainError);
        assert!(failure.message.contains("synthetic toolchain failure"));
        assert!(failure.message.contains('2'));
    }

    #[test]
    fn invalid_target_is_rejected() {
        let failure = target_from_raw(u32::MAX).expect_err("invalid target accepted");
        assert_eq!(failure.status, KumaStatus::UnsupportedTarget);
    }

    fn raw_target(target: Target) -> u32 {
        match target {
            Target::Amd64SysV => TARGET_AMD64_SYSV,
            Target::Amd64Apple => TARGET_AMD64_APPLE,
            Target::Aarch64Elf => TARGET_AARCH64_ELF,
            Target::Aarch64Apple => TARGET_AARCH64_APPLE,
        }
    }

    fn buffer_text(buffer: &KumaBuffer) -> String {
        if buffer.data.is_null() {
            return String::new();
        }
        // SAFETY: tests only inspect buffers returned by the C interface.
        let bytes = unsafe { std::slice::from_raw_parts(buffer.data, buffer.length) };
        String::from_utf8(bytes.to_vec()).expect("buffer UTF-8")
    }

    #[test]
    fn compile_returns_owned_assembly_buffer() {
        let source = b"export function w $main() {\n@start\n ret 0\n}\n";
        let mut output = KumaBuffer::empty();
        let mut error = KumaBuffer::empty();
        let status = kuma_compile(
            source.as_ptr(),
            source.len(),
            TARGET_AMD64_SYSV,
            &mut output,
            &mut error,
        );
        assert_eq!(status, KumaStatus::Success);
        assert!(buffer_text(&output).contains("main"));
        assert!(buffer_text(&error).is_empty());
        kuma_buffer_free(&mut output);
        kuma_buffer_free(&mut output);
        kuma_buffer_free(&mut error);
        assert!(output.data.is_null());
    }

    #[test]
    fn compile_rejects_aliased_output_buffers() {
        let source = b"function $f() {\n@start\n ret\n}\n";
        let mut buffer = KumaBuffer::empty();
        let pointer = &mut buffer as *mut KumaBuffer;
        let status = kuma_compile(
            source.as_ptr(),
            source.len(),
            TARGET_AMD64_SYSV,
            pointer,
            pointer,
        );
        assert_eq!(status, KumaStatus::InvalidArgument);
        assert!(buffer_text(&buffer).contains("distinct"));
        kuma_buffer_free(&mut buffer);
    }

    #[test]
    fn compile_reports_invalid_utf8() {
        let source = [0xff];
        let mut output = KumaBuffer::empty();
        let mut error = KumaBuffer::empty();
        let status = kuma_compile(
            source.as_ptr(),
            source.len(),
            TARGET_AMD64_SYSV,
            &mut output,
            &mut error,
        );
        assert_eq!(status, KumaStatus::InvalidUtf8);
        assert!(buffer_text(&error).contains("UTF-8"));
        kuma_buffer_free(&mut error);
    }

    #[test]
    fn compile_reports_parse_errors() {
        let source = b"function";
        let mut output = KumaBuffer::empty();
        let mut error = KumaBuffer::empty();
        let status = kuma_compile(
            source.as_ptr(),
            source.len(),
            TARGET_AMD64_SYSV,
            &mut output,
            &mut error,
        );
        assert_eq!(status, KumaStatus::ParseError);
        assert!(buffer_text(&error).contains("parse error"));
        kuma_buffer_free(&mut error);
    }

    #[test]
    fn compile_rejects_null_input() {
        let mut output = KumaBuffer::empty();
        let mut error = KumaBuffer::empty();
        let status = kuma_compile(
            std::ptr::null(),
            0,
            TARGET_AMD64_SYSV,
            &mut output,
            &mut error,
        );
        assert_eq!(status, KumaStatus::InvalidArgument);
        assert!(buffer_text(&error).contains("null"));
        kuma_buffer_free(&mut error);
    }

    #[test]
    fn assemble_rejects_cross_target() {
        let Some(host) = host_target() else { return };
        let cross = match host {
            Target::Amd64SysV | Target::Amd64Apple => Target::Aarch64Elf,
            Target::Aarch64Elf | Target::Aarch64Apple => Target::Amd64SysV,
        };
        let source = b"function $f() {\n@start\n ret\n}\n";
        let output = CString::new("unused.o").expect("C string");
        let mut error = KumaBuffer::empty();
        let status = kuma_assemble(
            source.as_ptr(),
            source.len(),
            raw_target(cross),
            output.as_ptr(),
            &mut error,
        );
        assert_eq!(status, KumaStatus::UnsupportedTarget);
        kuma_buffer_free(&mut error);
    }

    #[test]
    fn host_assemble_smoke_test() {
        let Some(host) = host_target() else { return };
        let source = b"export function w $main() {\n@start\n ret 0\n}\n";
        let path = std::env::temp_dir().join(format!("kuma-ffi-assemble-{}.o", std::process::id()));
        let output = CString::new(path.to_string_lossy().as_bytes()).expect("C path");
        let mut error = KumaBuffer::empty();
        let status = kuma_assemble(
            source.as_ptr(),
            source.len(),
            raw_target(host),
            output.as_ptr(),
            &mut error,
        );
        assert_eq!(status, KumaStatus::Success, "{}", buffer_text(&error));
        assert!(path.exists());
        let _ = std::fs::remove_file(path);
        kuma_buffer_free(&mut error);
    }

    #[test]
    fn host_link_smoke_test() {
        let Some(host) = host_target() else { return };
        let source = b"export function w $main() {\n@start\n ret 0\n}\n";
        let path = std::env::temp_dir().join(format!("kuma-ffi-link-{}", std::process::id()));
        let output = CString::new(path.to_string_lossy().as_bytes()).expect("C path");
        let mut error = KumaBuffer::empty();
        let status = kuma_compile_and_link(
            source.as_ptr(),
            source.len(),
            raw_target(host),
            output.as_ptr(),
            std::ptr::null(),
            0,
            &mut error,
        );
        assert_eq!(status, KumaStatus::Success, "{}", buffer_text(&error));
        assert!(path.exists());
        let _ = std::fs::remove_file(path);
        kuma_buffer_free(&mut error);
    }
}
