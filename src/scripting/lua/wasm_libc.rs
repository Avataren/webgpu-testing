//! The host half of the C library that Lua is built against on wasm32-unknown-unknown.
//!
//! That target has no libc, so the vendored `lua-src` (`third_party/lua-src`) compiles
//! Lua with a minimal one (`wasm/libc.c`). This module provides the parts that need
//! Rust's allocator, exact float conversion or the browser: `malloc` and friends,
//! float formatting/parsing, console output, the clock and aborting.

use std::alloc::{self, Layout};
use std::ffi::{c_char, c_int, c_void};
use std::ptr;
use std::sync::{Mutex, OnceLock};

/// Each allocation starts with a header holding its size, so `free` can rebuild the
/// layout. 16 bytes keeps the returned pointer aligned for any C type.
const HEADER: usize = 16;

fn layout(size: usize) -> Option<Layout> {
    Layout::from_size_align(size.checked_add(HEADER)?, HEADER).ok()
}

#[no_mangle]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    let Some(layout) = layout(size) else {
        return ptr::null_mut();
    };
    let base = alloc::alloc(layout);
    if base.is_null() {
        return ptr::null_mut();
    }
    base.cast::<usize>().write(size);
    base.add(HEADER).cast()
}

#[no_mangle]
pub unsafe extern "C" fn calloc(n: usize, size: usize) -> *mut c_void {
    let Some(layout) = n.checked_mul(size).and_then(layout) else {
        return ptr::null_mut();
    };
    let base = alloc::alloc_zeroed(layout);
    if base.is_null() {
        return ptr::null_mut();
    }
    base.cast::<usize>().write(n * size);
    base.add(HEADER).cast()
}

#[no_mangle]
pub unsafe extern "C" fn free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    let base = ptr.cast::<u8>().sub(HEADER);
    let size = base.cast::<usize>().read();
    alloc::dealloc(base, layout(size).unwrap());
}

#[no_mangle]
pub unsafe extern "C" fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    if ptr.is_null() {
        return malloc(size);
    }
    if layout(size).is_none() {
        return ptr::null_mut();
    }
    let base = ptr.cast::<u8>().sub(HEADER);
    let old = base.cast::<usize>().read();
    let base = alloc::realloc(base, layout(old).unwrap(), size + HEADER);
    if base.is_null() {
        return ptr::null_mut();
    }
    base.cast::<usize>().write(size);
    base.add(HEADER).cast()
}

/// Formats a finite, non-negative `v` like C's `%.*e` (`conv == 'e'`) or `%.*f`,
/// with Rust's exponent syntax (`1.5e-7`); libc.c rewrites the exponent and builds
/// `%g` on top. Writes at most `cap - 1` bytes plus a NUL and returns the length.
#[no_mangle]
pub unsafe extern "C" fn lua_wasm_fmt_f64(
    out: *mut c_char,
    cap: usize,
    v: f64,
    prec: c_int,
    conv: c_int,
) -> usize {
    let prec = prec.max(0) as usize;
    let text = if conv == c_int::from(b'e') {
        format!("{v:.prec$e}")
    } else {
        format!("{v:.prec$}")
    };
    if cap == 0 {
        return 0;
    }
    let n = text.len().min(cap - 1);
    ptr::copy_nonoverlapping(text.as_ptr(), out.cast::<u8>(), n);
    out.add(n).write(0);
    n
}

/// Parses a decimal float that libc.c's `strtod` already validated.
#[no_mangle]
pub unsafe extern "C" fn lua_wasm_parse_f64(s: *const c_char, len: usize) -> f64 {
    let bytes = std::slice::from_raw_parts(s.cast::<u8>(), len);
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0)
}

/// Pending partial lines for stdout (index 0) and stderr (index 1).
static OUTPUT: Mutex<[Vec<u8>; 2]> = Mutex::new([Vec::new(), Vec::new()]);

fn emit_line(fd: c_int, line: &[u8]) {
    let text = String::from_utf8_lossy(line);
    if fd == 2 {
        log::warn!(target: "lua", "{text}");
    } else {
        log::info!(target: "lua", "{text}");
    }
}

/// Writes to stdout (`fd` 1) or stderr (2), one log record per line.
#[no_mangle]
pub unsafe extern "C" fn lua_wasm_write(fd: c_int, s: *const c_char, len: usize) {
    let bytes = std::slice::from_raw_parts(s.cast::<u8>(), len);
    let mut output = OUTPUT.lock().unwrap_or_else(|e| e.into_inner());
    let pending = &mut output[usize::from(fd == 2)];
    pending.extend_from_slice(bytes);
    while let Some(end) = pending.iter().position(|&b| b == b'\n') {
        emit_line(fd, &pending[..end]);
        pending.drain(..=end);
    }
}

#[no_mangle]
pub extern "C" fn lua_wasm_flush(fd: c_int) {
    let mut output = OUTPUT.lock().unwrap_or_else(|e| e.into_inner());
    let pending = &mut output[usize::from(fd == 2)];
    if !pending.is_empty() {
        emit_line(fd, pending);
        pending.clear();
    }
}

#[no_mangle]
pub extern "C" fn lua_wasm_time_ms() -> f64 {
    js_sys::Date::now()
}

/// Milliseconds since the first call; backs `clock()` and so `os.clock()`.
#[no_mangle]
pub extern "C" fn lua_wasm_clock_ms() -> f64 {
    static START: OnceLock<f64> = OnceLock::new();
    js_sys::Date::now() - *START.get_or_init(js_sys::Date::now)
}

#[no_mangle]
pub unsafe extern "C" fn lua_wasm_abort(msg: *const c_char) -> ! {
    let msg = std::ffi::CStr::from_ptr(msg).to_string_lossy();
    panic!("Lua C runtime: {msg}");
}
