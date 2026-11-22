// SPDX-License-Identifier: MIT

//! Runtime `dlopen` fallback backend for ALSA PCM (`libasound.so.2`).

#![expect(
    unsafe_code,
    reason = "dlopen/dlsym FFI to libasound.so.2 with zero build-time C header dependency"
)]

use std::{
    ffi::{CStr, c_char, c_int, c_long, c_uint, c_ulong, c_void},
    ptr,
    sync::OnceLock,
};

use super::{BackendKind, PcmSpec, SampleFormat, backend::AudioBackend};

const LIB_NAME: &CStr = c"libasound.so.2";
const DEFAULT_DEVICE: &CStr = c"default";

/// `SND_PCM_STREAM_PLAYBACK` from `<alsa/pcm.h>`.
const SND_PCM_STREAM_PLAYBACK: c_int = 0;
/// `SND_PCM_ACCESS_RW_INTERLEAVED` from `<alsa/pcm.h>`.
const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;
/// `SND_PCM_FORMAT_S16_LE` from `<alsa/pcm.h>`.
const SND_PCM_FORMAT_S16_LE: c_int = 2;
/// `SND_PCM_FORMAT_FLOAT_LE` from `<alsa/pcm.h>`.
const SND_PCM_FORMAT_FLOAT_LE: c_int = 14;
/// Target ALSA playback latency in microseconds (60 ms).
const TARGET_LATENCY_US: c_uint = 60_000;
/// Maximum consecutive ALSA recovery attempts per write before failing over to the silent backend.
const MAX_RECOVERIES_PER_WRITE: usize = 3;

#[repr(C)]
struct SndPcm {
    _opaque: [u8; 0],
}

type SndPcmOpenFn = unsafe extern "C" fn(
    pcm: *mut *mut SndPcm,
    name: *const c_char,
    stream: c_int,
    mode: c_int,
) -> c_int;
type SndPcmCloseFn = unsafe extern "C" fn(pcm: *mut SndPcm) -> c_int;
type SndPcmSetParamsFn = unsafe extern "C" fn(
    pcm: *mut SndPcm,
    format: c_int,
    access: c_int,
    channels: c_uint,
    rate: c_uint,
    soft_resample: c_int,
    latency: c_uint,
) -> c_int;
type SndPcmWriteiFn =
    unsafe extern "C" fn(pcm: *mut SndPcm, buffer: *const c_void, size: c_ulong) -> c_long;
type SndPcmDelayFn = unsafe extern "C" fn(pcm: *mut SndPcm, delayp: *mut c_long) -> c_int;
type SndPcmDropFn = unsafe extern "C" fn(pcm: *mut SndPcm) -> c_int;
type SndPcmPrepareFn = unsafe extern "C" fn(pcm: *mut SndPcm) -> c_int;
type SndPcmRecoverFn = unsafe extern "C" fn(pcm: *mut SndPcm, err: c_int, silent: c_int) -> c_int;

struct AlsaApi {
    pcm_open: SndPcmOpenFn,
    pcm_close: SndPcmCloseFn,
    pcm_set_params: SndPcmSetParamsFn,
    pcm_writei: SndPcmWriteiFn,
    pcm_delay: SndPcmDelayFn,
    pcm_drop: SndPcmDropFn,
    pcm_prepare: SndPcmPrepareFn,
    pcm_recover: SndPcmRecoverFn,
}

// SAFETY: Function pointers loaded from a process-resident shared library are immutable and safe to
// share across threads.
unsafe impl Send for AlsaApi {}
// SAFETY: Function pointers loaded from a process-resident shared library are immutable and safe to
// share across threads.
unsafe impl Sync for AlsaApi {}

fn api() -> Option<&'static AlsaApi> {
    static API: OnceLock<Option<AlsaApi>> = OnceLock::new();
    API.get_or_init(load_api).as_ref()
}

fn load_api() -> Option<AlsaApi> {
    // SAFETY: `LIB_NAME` is a valid NUL-terminated C string; keeping the handle loaded for the
    // process lifetime avoids destructor hazards on unload.
    let handle = unsafe { libc::dlopen(LIB_NAME.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
    if handle.is_null() {
        return None;
    }
    let resolved = (|| {
        // SAFETY: `handle` is valid and `snd_pcm_open` matches `<alsa/pcm.h>`.
        let pcm_open = unsafe { symbol(handle, c"snd_pcm_open")? };
        // SAFETY: `handle` is valid and `snd_pcm_close` matches `<alsa/pcm.h>`.
        let pcm_close = unsafe { symbol(handle, c"snd_pcm_close")? };
        // SAFETY: `handle` is valid and `snd_pcm_set_params` matches `<alsa/pcm.h>`.
        let pcm_set_params = unsafe { symbol(handle, c"snd_pcm_set_params")? };
        // SAFETY: `handle` is valid and `snd_pcm_writei` matches `<alsa/pcm.h>`.
        let pcm_writei = unsafe { symbol(handle, c"snd_pcm_writei")? };
        // SAFETY: `handle` is valid and `snd_pcm_delay` matches `<alsa/pcm.h>`.
        let pcm_delay = unsafe { symbol(handle, c"snd_pcm_delay")? };
        // SAFETY: `handle` is valid and `snd_pcm_drop` matches `<alsa/pcm.h>`.
        let pcm_drop = unsafe { symbol(handle, c"snd_pcm_drop")? };
        // SAFETY: `handle` is valid and `snd_pcm_prepare` matches `<alsa/pcm.h>`.
        let pcm_prepare = unsafe { symbol(handle, c"snd_pcm_prepare")? };
        // SAFETY: `handle` is valid and `snd_pcm_recover` matches `<alsa/pcm.h>`.
        let pcm_recover = unsafe { symbol(handle, c"snd_pcm_recover")? };
        Some(AlsaApi {
            pcm_open,
            pcm_close,
            pcm_set_params,
            pcm_writei,
            pcm_delay,
            pcm_drop,
            pcm_prepare,
            pcm_recover,
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

pub(super) struct AlsaBackend {
    api: &'static AlsaApi,
    pcm: *mut SndPcm,
    spec: PcmSpec,
}

// SAFETY: `snd_pcm_t` is exclusively owned by the single audio worker thread that opens, writes,
// queries, flushes, and closes it.
unsafe impl Send for AlsaBackend {}

impl AlsaBackend {
    pub(super) fn open(spec: PcmSpec) -> Result<Self, ()> {
        let api = api().ok_or(())?;
        let mut pcm: *mut SndPcm = ptr::null_mut();
        // SAFETY: `pcm` out-pointer and `DEFAULT_DEVICE` C string are valid.
        let opened = unsafe {
            (api.pcm_open)(
                &mut pcm,
                DEFAULT_DEVICE.as_ptr(),
                SND_PCM_STREAM_PLAYBACK,
                0,
            )
        };
        if opened < 0 || pcm.is_null() {
            return Err(());
        }
        let format = match spec.format {
            SampleFormat::S16Le => SND_PCM_FORMAT_S16_LE,
            SampleFormat::F32Le => SND_PCM_FORMAT_FLOAT_LE,
        };
        // SAFETY: `pcm` is an open ALSA playback handle.
        let configured = unsafe {
            (api.pcm_set_params)(
                pcm,
                format,
                SND_PCM_ACCESS_RW_INTERLEAVED,
                c_uint::from(spec.channels),
                spec.sample_rate as c_uint,
                1,
                TARGET_LATENCY_US,
            )
        };
        if configured < 0 {
            // SAFETY: `pcm` was opened above and must be closed on configuration failure.
            unsafe {
                (api.pcm_close)(pcm);
            }
            return Err(());
        }
        Ok(Self { api, pcm, spec })
    }

    fn recover_write(&mut self, written: c_long, recoveries: &mut usize) -> Result<(), ()> {
        if written == 0 || *recoveries >= MAX_RECOVERIES_PER_WRITE {
            return Err(());
        }
        *recoveries += 1;
        let err = c_int::try_from(written).map_err(|_| ())?;
        // SAFETY: `self.pcm` is open; `snd_pcm_recover` attempts XRUN/suspend recovery.
        let recovered = unsafe { (self.api.pcm_recover)(self.pcm, err, 1) };
        if recovered < 0 { Err(()) } else { Ok(()) }
    }
}

impl AudioBackend for AlsaBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Alsa
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), ()> {
        let frame_bytes = self.spec.frame_bytes().max(1);
        let mut offset = 0;
        let mut recoveries = 0;
        while offset < bytes.len() {
            let frames = (bytes.len() - offset) / frame_bytes;
            if frames == 0 {
                break;
            }
            // SAFETY: `self.pcm` is open and `bytes[offset..]` holds at least `frames` frames.
            let written = unsafe {
                (self.api.pcm_writei)(self.pcm, bytes[offset..].as_ptr().cast(), frames as c_ulong)
            };
            if written > 0 {
                let advanced = usize::try_from(written).unwrap_or(frames).min(frames);
                offset += advanced * frame_bytes;
                recoveries = 0;
            } else {
                self.recover_write(written, &mut recoveries)?;
            }
        }
        Ok(())
    }

    fn latency_us(&mut self) -> Option<u64> {
        if self.spec.sample_rate == 0 {
            return None;
        }
        let mut delay: c_long = 0;
        // SAFETY: `self.pcm` is open and `delay` is a valid out-pointer.
        let rc = unsafe { (self.api.pcm_delay)(self.pcm, &mut delay) };
        if rc < 0 || delay < 0 {
            return None;
        }
        let delay_frames = u64::try_from(delay).ok()?;
        let us = u128::from(delay_frames) * 1_000_000 / u128::from(self.spec.sample_rate);
        u64::try_from(us).ok()
    }

    fn flush(&mut self) -> Result<(), ()> {
        // SAFETY: `self.pcm` is open; `snd_pcm_drop` discards queued frames immediately.
        let dropped = unsafe { (self.api.pcm_drop)(self.pcm) };
        // SAFETY: `self.pcm` is open; `snd_pcm_prepare` readies the stream after drop.
        let prepared = unsafe { (self.api.pcm_prepare)(self.pcm) };
        if dropped < 0 || prepared < 0 {
            Err(())
        } else {
            Ok(())
        }
    }
}

impl Drop for AlsaBackend {
    fn drop(&mut self) {
        // SAFETY: `self.pcm` is exclusively owned and closed once on drop.
        unsafe {
            (self.api.pcm_close)(self.pcm);
        }
    }
}
