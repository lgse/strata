// SPDX-License-Identifier: MIT

#![allow(
    unsafe_code,
    reason = "UnRAR's safe wrapper cannot stream member output; keep FFI confined to this sandboxed child"
)]

//! UnRAR FFI used by the sandboxed extraction helper.

use std::{ffi::CString, io::Write, os::unix::ffi::OsStrExt, path::Path, ptr};

use unrar_sys as native;

use crate::rar_extraction as wire;

#[cfg(test)]
mod tests;

// UnRAR 7.01 defines these in dll.hpp, but unrar_sys 0.5.8 omits them.
const UCM_LARGEDICT: native::UINT = 5;
const ERAR_LARGE_DICT: i32 = 25;
const LARGE_DICTIONARY: &str = "RAR dictionary exceeds the decoder's memory limit";
const INVALID_ARCHIVE: &str = "This file is not a valid archive or is damaged.";
const MAYBE_BAD_PASSWORD: &str = "The password may be incorrect.";

struct Archive(*const native::Handle);

impl Drop for Archive {
    fn drop(&mut self) {
        // SAFETY: The handle is owned here and all synchronous callbacks have returned.
        unsafe {
            native::RARCloseArchive(self.0);
        }
    }
}

type MemberSink<'a> = dyn FnMut(&[u8]) -> Result<(), String> + 'a;

struct CallbackState<'a, 'b> {
    sink: Option<&'a mut MemberSink<'b>>,
    password: Option<&'a str>,
    error: Option<String>,
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
        match message {
            UCM_LARGEDICT => return Err(LARGE_DICTIONARY.to_owned()),
            native::UCM_PROCESSDATA => {
                if p2 < 0 || (p1 == 0 && p2 != 0) {
                    return Err("Invalid RAR output buffer".to_owned());
                }
                if p2 != 0
                    && let Some(sink) = state.sink.as_mut()
                {
                    // SAFETY: The library owns this readable buffer until the callback returns.
                    sink(unsafe { std::slice::from_raw_parts(p1 as *const u8, p2 as usize) })?;
                }
            }
            native::UCM_NEEDPASSWORD | native::UCM_NEEDPASSWORDW => {
                let password = state
                    .password
                    .ok_or_else(|| "A password is required to extract this archive.".to_owned())?;
                if p1 == 0 || p2 <= 0 {
                    return Err("Invalid RAR password buffer".to_owned());
                }
                if message == native::UCM_NEEDPASSWORDW {
                    let chars: Vec<native::WCHAR> = password
                        .chars()
                        .map(|ch| ch as native::WCHAR)
                        .chain([0])
                        .collect();
                    if chars.len() > p2 as usize {
                        return Err("RAR password is too long".to_owned());
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
                    let password = CString::new(password).map_err(|error| error.to_string())?;
                    let bytes = password.as_bytes_with_nul();
                    if bytes.len() > p2 as usize {
                        return Err("RAR password is too long".to_owned());
                    }
                    // SAFETY: UnRAR supplies p2 writable bytes, including the terminator.
                    unsafe {
                        ptr::copy_nonoverlapping(bytes.as_ptr(), p1 as *mut u8, bytes.len());
                    }
                }
            }
            native::UCM_CHANGEVOLUME | native::UCM_CHANGEVOLUMEW if p2 == native::RAR_VOL_ASK => {
                return Err("The next RAR volume is missing".to_owned());
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
            state.error = Some("RAR output callback failed".to_owned());
            -1
        }
    }
}

fn call<'a, 'b>(
    password: Option<&'a str>,
    sink: Option<&'a mut MemberSink<'b>>,
    invoke: impl FnOnce(native::LPARAM) -> i32,
) -> Result<i32, String> {
    let mut state = CallbackState {
        sink,
        password,
        error: None,
    };
    let code = invoke(&mut state as *mut _ as native::LPARAM);
    match state.error {
        Some(error) => Err(error),
        None => Ok(code),
    }
}

fn decode_result(code: i32, password: Option<&str>) -> Result<(), String> {
    if code == native::ERAR_SUCCESS {
        return Ok(());
    }
    if code == ERAR_LARGE_DICT {
        return Err(LARGE_DICTIONARY.to_owned());
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

fn unrar_decode_error(error: unrar::error::UnrarError, password_supplied: bool) -> String {
    use unrar::error::Code;
    match error.code {
        Code::MissingPassword => "A password is required to extract this archive.".to_owned(),
        Code::BadPassword => MAYBE_BAD_PASSWORD.to_owned(),
        Code::BadData if password_supplied => MAYBE_BAD_PASSWORD.to_owned(),
        Code::BadArchive | Code::UnknownFormat | Code::BadData => INVALID_ARCHIVE.to_owned(),
        _ => error.to_string(),
    }
}

pub(super) fn run(
    archive_path: &Path,
    password: Option<&str>,
    writer: &mut impl Write,
) -> Result<(), String> {
    wire::write_magic(writer).map_err(|error| error.to_string())?;
    match extract(archive_path, password, writer) {
        Ok(()) => Ok(()),
        Err(error) => {
            wire::write_error(writer, &error).map_err(|error| error.to_string())?;
            Err(error)
        }
    }
}

fn extract(
    archive_path: &Path,
    password: Option<&str>,
    writer: &mut impl Write,
) -> Result<(), String> {
    let path =
        CString::new(archive_path.as_os_str().as_bytes()).map_err(|error| error.to_string())?;
    if password.is_some_and(|value| value.contains('\0')) {
        return Err("RAR password contains a NUL character".to_owned());
    }
    let mut handle = ptr::null();
    let opened = call(password, None, |user| {
        let mut data = native::OpenArchiveDataEx::new(path.as_ptr(), native::RAR_OM_EXTRACT);
        data.callback = Some(callback);
        data.user_data = user;
        // SAFETY: The path and callback state outlive this call; data is exclusively writable.
        unsafe {
            handle = native::RAROpenArchiveEx(&raw mut data);
        }
        data.open_result as i32
    })?;
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
    decode_result(opened, password)?;
    let archive = archive.ok_or_else(|| "Unable to open RAR archive".to_owned())?;
    loop {
        let mut header = native::HeaderDataEx::default();
        let code = call(password, None, |user| {
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
            wire::write_end(writer).map_err(|error| error.to_string())?;
            return Ok(());
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
        let process = |sink: &mut MemberSink<'_>| {
            let code = call(password, Some(sink), |user| {
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
            wire::write_directory(writer, &name).map_err(|error| error.to_string())?;
            process(&mut |_| Ok(()))?;
        } else {
            wire::write_file_header(writer, &name, size).map_err(|error| error.to_string())?;
            let mut written = 0u64;
            let outcome = process(&mut |bytes| {
                let length = bytes.len() as u64;
                if length > size.saturating_sub(written) {
                    return Err(format!(
                        "Archive member `{name}` declared {size} bytes but produced more"
                    ));
                }
                wire::write_chunk(writer, bytes).map_err(|error| error.to_string())?;
                written += length;
                Ok(())
            });
            let outcome = outcome.and_then(|()| {
                if written == size {
                    Ok(())
                } else {
                    Err(format!(
                        "Archive member `{name}` declared {size} bytes but produced {written} bytes"
                    ))
                }
            });
            match outcome {
                Ok(()) => wire::write_file_ok(writer).map_err(|error| error.to_string())?,
                Err(message) => {
                    // The trailer reports this error; a second error record would desynchronize the stream.
                    wire::write_file_failed(writer, &message).map_err(|error| error.to_string())?;
                    return Ok(());
                }
            }
        }
    }
}

pub(super) fn cover_image(path: &Path) -> Result<Vec<u8>, String> {
    use crate::sandbox_helper::archive_cover::{
        MAX_ENTRIES, MAX_IMAGE_BYTES, compare_names, image_name,
    };

    let path = CString::new(path.as_os_str().as_bytes()).map_err(|error| error.to_string())?;
    let mut selected: Option<String> = None;
    for scanning in [true, false] {
        let mut handle = ptr::null();
        let opened = call(None, None, |user| {
            let mut data = native::OpenArchiveDataEx::new(path.as_ptr(), native::RAR_OM_EXTRACT);
            data.callback = Some(callback);
            data.user_data = user;
            // SAFETY: The path and synchronous callback state outlive this call.
            unsafe {
                handle = native::RAROpenArchiveEx(&raw mut data);
            }
            data.open_result as i32
        });
        let archive = (!handle.is_null()).then_some(Archive(handle));
        if let Some(archive) = &archive {
            // SAFETY: The handle is live; remove the expired callback state.
            unsafe {
                native::RARSetCallback(archive.0, None, 0);
            }
        }
        decode_result(opened?, None)?;
        let archive = archive.ok_or("Unable to open RAR archive")?;
        let mut complete = false;
        for _ in 0..MAX_ENTRIES {
            let mut header = native::HeaderDataEx::default();
            let code = read_cover_header(&archive, &mut header)?;
            if code == native::ERAR_END_ARCHIVE {
                if scanning {
                    complete = true;
                    break;
                }
                return Err("Comic cover is missing".into());
            }
            decode_result(code, None)?;
            let name: String = header
                .filename_w
                .iter()
                .take_while(|c| **c != 0)
                .map(|c| char::from_u32(*c as u32).unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect();
            let size = u64::from(header.unp_size) | (u64::from(header.unp_size_high) << 32);
            let candidate = header.flags & native::RHDF_DIRECTORY == 0 && image_name(&name);
            if scanning
                && candidate
                && selected
                    .as_deref()
                    .is_none_or(|best| compare_names(&name, best).is_lt())
            {
                selected = Some(name.clone());
            }
            let chosen = !scanning && selected.as_deref() == Some(&name);
            if chosen && size > MAX_IMAGE_BYTES {
                return Err("Comic cover exceeds the size limit".into());
            }
            let mut bytes = Vec::new();
            let code = call(
                None,
                Some(&mut |chunk: &[u8]| {
                    if bytes.len() as u64 + chunk.len() as u64 > MAX_IMAGE_BYTES {
                        return Err("Comic cover exceeds the size limit".into());
                    }
                    bytes.extend_from_slice(chunk);
                    Ok(())
                }),
                |user| {
                    // SAFETY: The handle and callback state remain live throughout this call.
                    unsafe { native::RARSetCallback(archive.0, Some(callback), user) };
                    // SAFETY: The handle and callback state remain live until processing returns.
                    let code = unsafe {
                        native::RARProcessFile(
                            archive.0,
                            if chosen {
                                native::RAR_TEST
                            } else {
                                native::RAR_SKIP
                            },
                            ptr::null(),
                            ptr::null(),
                        )
                    };
                    // SAFETY: The handle is live; clear the callback before its state expires.
                    unsafe { native::RARSetCallback(archive.0, None, 0) };
                    code
                },
            )?;
            decode_result(code, None)?;
            if chosen {
                if bytes.len() as u64 != size {
                    return Err("Comic cover size mismatch".into());
                }
                return Ok(bytes);
            }
        }
        if scanning && selected.is_none() {
            return Err("Comic archive has no bounded image".into());
        }
        if scanning && !complete {
            // A bounded scan may stop before the end; reject rather than choose a partial listing.
            let mut header = native::HeaderDataEx::default();
            let code = read_cover_header(&archive, &mut header)?;
            if code != native::ERAR_END_ARCHIVE {
                return Err("Comic archive entry limit exceeded".into());
            }
        }
    }
    Err("Comic cover is missing".into())
}

fn read_cover_header(archive: &Archive, header: &mut native::HeaderDataEx) -> Result<i32, String> {
    call(None, None, |user| {
        // SAFETY: The handle, header and callback state remain live for this call.
        unsafe { native::RARSetCallback(archive.0, Some(callback), user) };
        // SAFETY: The handle and exclusive header remain live until this read returns.
        let code = unsafe { native::RARReadHeaderEx(archive.0, header) };
        // SAFETY: The handle is live; clear the callback before its state expires.
        unsafe { native::RARSetCallback(archive.0, None, 0) };
        code
    })
}
