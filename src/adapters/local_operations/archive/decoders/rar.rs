// SPDX-License-Identifier: MIT

#![allow(
    unsafe_code,
    reason = "UnRAR's safe wrapper cannot stream or cancel member output; keep FFI confined to this adapter"
)]

use super::super::{check_archive_cancelled, extraction::MemberSink};
use super::{
    ArchiveError, ArchiveOutcome, ExtractionSession, MemberContent, archive_failed,
    unrar_decode_error,
};
use std::{
    ffi::CString,
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    },
};
use unrar_sys as native;

#[cfg(test)]
mod tests;

// UnRAR 7.01 defines these in dll.hpp, but unrar_sys 0.5.8 omits them.
const UCM_LARGEDICT: native::UINT = 5;
const ERAR_LARGE_DICT: i32 = 25;
const LARGE_DICTIONARY: &str = "RAR dictionary exceeds the decoder's memory limit";

struct Archive(*const native::Handle);

impl Drop for Archive {
    fn drop(&mut self) {
        // SAFETY: The handle is owned here and all synchronous callbacks have returned.
        unsafe {
            native::RARCloseArchive(self.0);
        }
    }
}

struct CallbackState<'a, 'b> {
    sink: Option<&'a mut MemberSink<'b>>,
    password: Option<&'a str>,
    cancelled: &'a AtomicBool,
    error: Option<ArchiveError>,
}

extern "C" fn callback(
    message: native::UINT,
    user: native::LPARAM,
    p1: native::LPARAM,
    p2: native::LPARAM,
) -> i32 {
    // SAFETY: UnRAR invokes this synchronously with the live, exclusive state installed by call().
    let state = unsafe { &mut *(user as *mut CallbackState<'_, '_>) };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_archive_cancelled(state.cancelled)?;
        match message {
            UCM_LARGEDICT => return Err(archive_failed(LARGE_DICTIONARY)),
            native::UCM_PROCESSDATA => {
                if p2 < 0 || (p1 == 0 && p2 != 0) {
                    return Err(archive_failed("Invalid RAR output buffer"));
                }
                if p2 != 0
                    && let Some(sink) = state.sink.as_mut()
                {
                    // SAFETY: The library owns this readable buffer until the callback returns.
                    sink(unsafe { std::slice::from_raw_parts(p1 as *const u8, p2 as usize) })?;
                }
            }
            native::UCM_NEEDPASSWORD | native::UCM_NEEDPASSWORDW => {
                let password = state.password.ok_or_else(|| {
                    archive_failed("A password is required to extract this archive.")
                })?;
                if p1 == 0 || p2 <= 0 {
                    return Err(archive_failed("Invalid RAR password buffer"));
                }
                if message == native::UCM_NEEDPASSWORDW {
                    let chars: Vec<native::WCHAR> = password
                        .chars()
                        .map(|ch| ch as native::WCHAR)
                        .chain([0])
                        .collect();
                    if chars.len() > p2 as usize {
                        return Err(archive_failed("RAR password is too long"));
                    }
                    // SAFETY: UnRAR supplies p2 writable wide characters, including the terminator.
                    unsafe {
                        ptr::copy_nonoverlapping(
                            chars.as_ptr(),
                            p1 as *mut native::WCHAR,
                            chars.len(),
                        );
                    }
                } else {
                    let password = CString::new(password).map_err(archive_failed)?;
                    let bytes = password.as_bytes_with_nul();
                    if bytes.len() > p2 as usize {
                        return Err(archive_failed("RAR password is too long"));
                    }
                    // SAFETY: UnRAR supplies p2 writable bytes, including the terminator.
                    unsafe {
                        ptr::copy_nonoverlapping(bytes.as_ptr(), p1 as *mut u8, bytes.len());
                    }
                }
            }
            native::UCM_CHANGEVOLUME | native::UCM_CHANGEVOLUMEW if p2 == native::RAR_VOL_ASK => {
                return Err(archive_failed("The next RAR volume is missing"));
            }
            _ => {}
        }
        Ok(())
    }));
    match result {
        Ok(Ok(())) => 1,
        Ok(Err(error)) => {
            state.error = Some(error);
            -1
        }
        Err(_) => {
            state.error = Some(archive_failed("RAR output callback failed"));
            -1
        }
    }
}

fn call<'a, 'b>(
    password: Option<&'a str>,
    cancelled: &'a AtomicBool,
    sink: Option<&'a mut MemberSink<'b>>,
    invoke: impl FnOnce(native::LPARAM) -> i32,
) -> Result<i32, ArchiveError> {
    check_archive_cancelled(cancelled)?;
    let mut state = CallbackState {
        sink,
        password,
        cancelled,
        error: None,
    };
    let code = invoke(&mut state as *mut _ as native::LPARAM);
    match state.error {
        Some(error) => Err(error),
        None => Ok(code),
    }
}

fn decode_result(code: i32, password: Option<&str>) -> Result<(), ArchiveError> {
    if code == native::ERAR_SUCCESS {
        return Ok(());
    }
    if code == ERAR_LARGE_DICT {
        return Err(archive_failed(LARGE_DICTIONARY));
    }
    let code = unrar::error::Code::from(code).unwrap_or(unrar::error::Code::Unknown);
    Err(unrar_decode_error(
        unrar::error::UnrarError {
            code,
            when: unrar::error::When::Process,
        },
        password.is_some(),
    ))
}

pub(in crate::adapters::local_operations::archive) fn extract_rar(
    archive_path: &Path,
    dest_dir: &Path,
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<ArchiveOutcome<Option<String>>, ArchiveError> {
    let mut session = ExtractionSession::open(dest_dir, progress, cancelled)?;
    let result = (|| {
        let path = CString::new(archive_path.as_os_str().as_bytes()).map_err(archive_failed)?;
        if password.is_some_and(|value| value.contains('\0')) {
            return Err(archive_failed("RAR password contains a NUL character"));
        }
        let mut handle = ptr::null();
        let opened = call(password, cancelled, None, |user| {
            let mut data = native::OpenArchiveDataEx::new(path.as_ptr(), native::RAR_OM_EXTRACT);
            data.callback = Some(callback);
            data.user_data = user;
            // SAFETY: The path and callback state outlive this call; data is exclusively writable.
            unsafe {
                handle = native::RAROpenArchiveEx(&raw mut data);
            }
            data.open_result as i32
        });
        let archive = if handle.is_null() {
            None
        } else {
            Some(Archive(handle))
        };
        if let Some(archive) = &archive {
            // SAFETY: The handle is live; clear the expired stack callback before any further calls.
            unsafe {
                native::RARSetCallback(archive.0, None, 0);
            }
        }
        decode_result(opened?, password)?;
        let archive = archive.ok_or_else(|| archive_failed("Unable to open RAR archive"))?;
        loop {
            let mut header = native::HeaderDataEx::default();
            let code = call(password, cancelled, None, |user| {
                // SAFETY: The handle and exclusive callback state remain live throughout this call.
                unsafe {
                    native::RARSetCallback(archive.0, Some(callback), user);
                }
                // SAFETY: The header is exclusively writable until the native call returns.
                let result = unsafe { native::RARReadHeaderEx(archive.0, &raw mut header) };
                // SAFETY: The handle is live; clear the callback before its state expires.
                unsafe {
                    native::RARSetCallback(archive.0, None, 0);
                }
                result
            })?;
            if code == native::ERAR_END_ARCHIVE {
                break;
            }
            decode_result(code, password)?;
            let name: String = header
                .filename_w
                .iter()
                .take_while(|ch| **ch != 0)
                .map(|ch| char::from_u32(*ch as u32).unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect();
            let directory = header.flags & native::RHDF_DIRECTORY != 0;
            let size = u64::from(header.unp_size) | (u64::from(header.unp_size_high) << 32);
            let mut process = |sink: &mut MemberSink<'_>| {
                let code = call(password, cancelled, Some(sink), |user| {
                    // SAFETY: The handle and exclusive callback state remain live throughout this call.
                    unsafe {
                        native::RARSetCallback(archive.0, Some(callback), user);
                    }
                    // SAFETY: RAR_TEST only emits callback bytes; no destination pointers are needed.
                    let result = unsafe {
                        native::RARProcessFile(
                            archive.0,
                            if directory {
                                native::RAR_SKIP
                            } else {
                                native::RAR_TEST
                            },
                            ptr::null(),
                            ptr::null(),
                        )
                    };
                    // SAFETY: The handle is live; clear the callback before its state expires.
                    unsafe {
                        native::RARSetCallback(archive.0, None, 0);
                    }
                    result
                })?;
                decode_result(code, password)
            };
            if directory {
                session.extract_member(&name, MemberContent::Directory)?;
                process(&mut |_| Ok(()))?;
            } else {
                session.extract_member(&name, MemberContent::Decoded(&mut process, size))?;
            }
        }
        Ok(())
    })();
    session.finish(result, Vec::new)
}
