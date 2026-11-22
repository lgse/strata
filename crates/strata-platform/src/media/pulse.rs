// SPDX-License-Identifier: MIT

//! Runtime `dlopen` backend for `libpulse-simple.so.0` (PipeWire via `pipewire-pulse` and PulseAudio).

#![expect(
    unsafe_code,
    reason = "dlopen/dlsym FFI to libpulse-simple.so.0 with zero build-time C header dependency"
)]

use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    ptr,
    sync::OnceLock,
};

use super::{BackendKind, PcmSpec, SampleFormat, backend::AudioBackend};

const LIB_NAME: &CStr = c"libpulse-simple.so.0";

/// `pa_stream_direction_t::PA_STREAM_PLAYBACK` from `<pulse/def.h>`.
const PA_STREAM_PLAYBACK: c_int = 1;
/// `pa_sample_format_t::PA_SAMPLE_S16LE` from `<pulse/sample.h>`.
const PA_SAMPLE_S16LE: c_int = 3;
/// `pa_sample_format_t::PA_SAMPLE_FLOAT32LE` from `<pulse/sample.h>`.
const PA_SAMPLE_FLOAT32LE: c_int = 5;
/// Target server playback buffer length in milliseconds (low enough for crisp volume/pause).
const TARGET_LATENCY_MS: u64 = 60;
/// Minimum server request block in milliseconds.
const MIN_REQUEST_MS: u64 = 15;

#[repr(C)]
struct PaSimple {
    _opaque: [u8; 0],
}

#[repr(C)]
struct PaSampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

#[repr(C)]
struct PaBufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

type PaSimpleNewFn = unsafe extern "C" fn(
    server: *const c_char,
    name: *const c_char,
    dir: c_int,
    dev: *const c_char,
    stream_name: *const c_char,
    ss: *const PaSampleSpec,
    map: *const c_void,
    attr: *const PaBufferAttr,
    error: *mut c_int,
) -> *mut PaSimple;
type PaSimpleFreeFn = unsafe extern "C" fn(s: *mut PaSimple);
type PaSimpleWriteFn = unsafe extern "C" fn(
    s: *mut PaSimple,
    data: *const c_void,
    bytes: usize,
    error: *mut c_int,
) -> c_int;
type PaSimpleGetLatencyFn = unsafe extern "C" fn(s: *mut PaSimple, error: *mut c_int) -> u64;
type PaSimpleFlushFn = unsafe extern "C" fn(s: *mut PaSimple, error: *mut c_int) -> c_int;

struct PulseApi {
    simple_new: PaSimpleNewFn,
    simple_free: PaSimpleFreeFn,
    simple_write: PaSimpleWriteFn,
    simple_get_latency: PaSimpleGetLatencyFn,
    simple_flush: PaSimpleFlushFn,
}

// SAFETY: Function pointers loaded from a process-resident shared library are immutable and safe to
// share across threads.
unsafe impl Send for PulseApi {}
// SAFETY: Function pointers loaded from a process-resident shared library are immutable and safe to
// share across threads.
unsafe impl Sync for PulseApi {}

fn api() -> Option<&'static PulseApi> {
    static API: OnceLock<Option<PulseApi>> = OnceLock::new();
    API.get_or_init(load_api).as_ref()
}

fn load_api() -> Option<PulseApi> {
    // SAFETY: `LIB_NAME` is a valid NUL-terminated C string; keeping the handle loaded for the
    // process lifetime avoids TLS/destructor hazards on unload.
    let handle = unsafe { libc::dlopen(LIB_NAME.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
    if handle.is_null() {
        return None;
    }
    let resolved = (|| {
        // SAFETY: `handle` is valid and `pa_simple_new` matches `<pulse/simple.h>`.
        let simple_new = unsafe { symbol(handle, c"pa_simple_new")? };
        // SAFETY: `handle` is valid and `pa_simple_free` matches `<pulse/simple.h>`.
        let simple_free = unsafe { symbol(handle, c"pa_simple_free")? };
        // SAFETY: `handle` is valid and `pa_simple_write` matches `<pulse/simple.h>`.
        let simple_write = unsafe { symbol(handle, c"pa_simple_write")? };
        // SAFETY: `handle` is valid and `pa_simple_get_latency` matches `<pulse/simple.h>`.
        let simple_get_latency = unsafe { symbol(handle, c"pa_simple_get_latency")? };
        // SAFETY: `handle` is valid and `pa_simple_flush` matches `<pulse/simple.h>`.
        let simple_flush = unsafe { symbol(handle, c"pa_simple_flush")? };
        Some(PulseApi {
            simple_new,
            simple_free,
            simple_write,
            simple_get_latency,
            simple_flush,
        })
    })();
    if resolved.is_none() {
        // SAFETY: `handle` is a valid non-null handle returned by `dlopen` above.
        unsafe {
            libc::dlclose(handle);
        }
    }
    resolved
}

/// Resolves a symbol from `handle` and transmutes it to `T`.
///
/// # Safety
/// `handle` must be a valid `dlopen` handle and `T` must match the C function signature of `name`.
unsafe fn symbol<T: Copy>(handle: *mut c_void, name: &CStr) -> Option<T> {
    // SAFETY: Caller guarantees `handle` is valid; `name` is NUL-terminated.
    let ptr = unsafe { libc::dlsym(handle, name.as_ptr()) };
    if ptr.is_null() {
        return None;
    }
    // SAFETY: Caller guarantees `T` matches the resolved function pointer ABI.
    Some(unsafe { std::mem::transmute_copy(&ptr) })
}

fn aligned_ms_bytes(spec: PcmSpec, ms: u64, min_frames: usize) -> u32 {
    let frame = spec.frame_bytes().max(1);
    let raw = (spec.bytes_per_second() * ms / 1_000) as usize;
    let aligned = (raw / frame).max(min_frames) * frame;
    u32::try_from(aligned).unwrap_or(u32::MAX - 1)
}

fn c_label(text: &str, fallback: &CStr) -> CString {
    let sanitized: String = text.chars().filter(|&ch| ch != '\0').collect();
    if sanitized.is_empty() {
        fallback.to_owned()
    } else {
        CString::new(sanitized).unwrap_or_else(|_| fallback.to_owned())
    }
}

pub(super) struct PulseBackend {
    api: &'static PulseApi,
    stream: *mut PaSimple,
}

// SAFETY: `pa_simple` is exclusively owned by the single audio worker thread that opens, writes,
// queries, flushes, and drops it.
unsafe impl Send for PulseBackend {}

impl PulseBackend {
    pub(super) fn open(spec: PcmSpec, app_name: &str, stream_name: &str) -> Result<Self, ()> {
        let api = api().ok_or(())?;
        let channels = u8::try_from(spec.channels).map_err(|_| ())?;
        let pa_spec = PaSampleSpec {
            format: match spec.format {
                SampleFormat::S16Le => PA_SAMPLE_S16LE,
                SampleFormat::F32Le => PA_SAMPLE_FLOAT32LE,
            },
            rate: spec.sample_rate,
            channels,
        };
        let attr = PaBufferAttr {
            maxlength: u32::MAX,
            tlength: aligned_ms_bytes(spec, TARGET_LATENCY_MS, 4),
            prebuf: u32::MAX,
            minreq: aligned_ms_bytes(spec, MIN_REQUEST_MS, 1),
            fragsize: u32::MAX,
        };
        let app = c_label(app_name, c"audio-app");
        let stream_desc = c_label(stream_name, c"playback");
        let mut error: c_int = 0;
        // SAFETY: All pointers passed to `pa_simple_new` are valid for the duration of the call.
        let stream = unsafe {
            (api.simple_new)(
                ptr::null(),
                app.as_ptr(),
                PA_STREAM_PLAYBACK,
                ptr::null(),
                stream_desc.as_ptr(),
                &pa_spec,
                ptr::null(),
                &attr,
                &mut error,
            )
        };
        if stream.is_null() {
            return Err(());
        }
        Ok(Self { api, stream })
    }
}

impl AudioBackend for PulseBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::PulseAudio
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), ()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let mut error: c_int = 0;
        // SAFETY: `self.stream` is a live `pa_simple` handle and `bytes` is readable for `bytes.len()`.
        let rc = unsafe {
            (self.api.simple_write)(self.stream, bytes.as_ptr().cast(), bytes.len(), &mut error)
        };
        if rc < 0 { Err(()) } else { Ok(()) }
    }

    fn latency_us(&mut self) -> Option<u64> {
        let mut error: c_int = 0;
        // SAFETY: `self.stream` is a live `pa_simple` handle.
        let latency = unsafe { (self.api.simple_get_latency)(self.stream, &mut error) };
        if latency == u64::MAX || error != 0 {
            None
        } else {
            Some(latency)
        }
    }

    fn flush(&mut self) -> Result<(), ()> {
        let mut error: c_int = 0;
        // SAFETY: `self.stream` is a live `pa_simple` handle.
        let rc = unsafe { (self.api.simple_flush)(self.stream, &mut error) };
        if rc < 0 { Err(()) } else { Ok(()) }
    }
}

impl Drop for PulseBackend {
    fn drop(&mut self) {
        // SAFETY: `self.stream` is exclusively owned and freed once on drop.
        unsafe {
            (self.api.simple_free)(self.stream);
        }
    }
}
