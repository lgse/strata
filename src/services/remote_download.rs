// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, SystemTime},
};

use super::file_source::io_error_message;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const BODY_TIMEOUT: Duration = Duration::from_secs(60);
const STALE_DOWNLOAD_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_NAME_BYTES: usize = 255;
const CHUNK: usize = 64 * 1024;
const PROGRESS_STRIDE: u64 = 256 * 1024;

pub(crate) const DOWNLOAD_PREFIX: &str = "strata-download-";

#[derive(Debug)]
pub(crate) enum RemoteDownload {
    Named(String),
    Progress { downloaded: u64, total: Option<u64> },
    Finished(PathBuf),
    Failed(String),
}

pub(crate) fn remote_file_url(input: &str) -> Option<String> {
    let input = input.trim();
    let (scheme, rest) = input.split_once("://")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return None;
    }
    match rest.split(['/', '?', '#']).next() {
        Some(host) if !host.is_empty() && !host.contains('@') => Some(input.to_owned()),
        _ => None,
    }
}

pub(crate) fn remote_file_name(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let path = rest.split_once('/')?.1;
    let path = path.split(['?', '#']).next()?;
    let last = path.rsplit('/').find(|segment| !segment.is_empty())?;
    sanitize_file_name(&percent_decode(last)?)
}

pub(crate) fn download_remote(url: String, cancelled: Arc<AtomicBool>) -> Receiver<RemoteDownload> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("strata-remote-download".into())
        .spawn(move || {
            let event = match fetch(&url, &cancelled, &sender) {
                Ok(path) => RemoteDownload::Finished(path),
                Err(message) => RemoteDownload::Failed(message),
            };
            let _sent = sender.send(event);
        })
        .expect("spawn download worker");
    receiver
}

fn fetch(
    url: &str,
    cancelled: &AtomicBool,
    progress: &Sender<RemoteDownload>,
) -> Result<PathBuf, String> {
    prune_stale_downloads();
    if cancelled.load(Ordering::SeqCst) {
        return Err(crate::i18n::tr("Download cancelled"));
    }
    let config = ureq::Agent::config_builder()
        // A remote server must not redirect the portal into host-only services.
        .max_redirects(0)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(RESPONSE_TIMEOUT))
        .timeout_recv_body(Some(BODY_TIMEOUT))
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent
        .get(url)
        .header("User-Agent", "strata-file-manager")
        .call()
        .map_err(|error| download_failed(&error.to_string()))?;
    if response.status().is_redirection() {
        return Err(crate::i18n::tr(
            "The URL redirects elsewhere; paste the direct file URL instead",
        ));
    }
    let disposition = response
        .headers()
        .get("content-disposition")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let name = disposition
        .as_deref()
        .and_then(filename_from_disposition)
        .or_else(|| remote_file_name(url))
        .unwrap_or_else(|| "download".to_owned());
    let _sent = progress.send(RemoteDownload::Named(name.clone()));
    let total = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let directory = tempfile::Builder::new()
        .prefix(DOWNLOAD_PREFIX)
        .tempdir()
        .map_err(|error| {
            rust_i18n::t!(
                "Could not create a temporary folder: %{error}",
                error = io_error_message(&error)
            )
            .into_owned()
        })?;
    let path = directory.path().join(&name);
    let mut file = fs::File::create(&path).map_err(|error| write_failed(&error))?;
    let mut reader = response.body_mut().as_reader();
    let mut buffer = [0_u8; CHUNK];
    let mut downloaded = 0_u64;
    let mut reported = 0_u64;
    let _sent = progress.send(RemoteDownload::Progress { downloaded, total });
    loop {
        if cancelled.load(Ordering::SeqCst) {
            return Err(crate::i18n::tr("Download cancelled"));
        }
        let count = reader
            .read(&mut buffer)
            .map_err(|error| download_failed(&error.to_string()))?;
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count])
            .map_err(|error| write_failed(&error))?;
        downloaded = downloaded.saturating_add(count as u64);
        if downloaded - reported >= PROGRESS_STRIDE {
            reported = downloaded;
            let _sent = progress.send(RemoteDownload::Progress { downloaded, total });
        }
    }
    if cancelled.load(Ordering::SeqCst) {
        return Err(crate::i18n::tr("Download cancelled"));
    }
    // The requesting app opens the file after the portal request completes.
    let _persisted = directory.keep();
    Ok(path)
}

fn download_failed(error: &str) -> String {
    rust_i18n::t!("Could not download the file: %{error}", error = error).into_owned()
}

fn write_failed(error: &std::io::Error) -> String {
    rust_i18n::t!(
        "Could not write the download: %{error}",
        error = io_error_message(error)
    )
    .into_owned()
}

fn filename_from_disposition(header: &str) -> Option<String> {
    let mut plain = None;
    for part in split_header_params(header) {
        let Some((name, value)) = part.split_once('=') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = unquote(value.trim());
        if name == "filename*" {
            // RFC 5987: charset'lang'percent-encoded
            if let Some(name) = value
                .splitn(3, '\'')
                .nth(2)
                .and_then(percent_decode)
                .and_then(|name| sanitize_file_name(&name))
            {
                return Some(name);
            }
        } else if name == "filename" {
            plain = sanitize_file_name(&value);
        }
    }
    plain
}

fn split_header_params(header: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    for (index, byte) in header.bytes().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b';' if !quoted => {
                parts.push(&header[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&header[start..]);
    parts
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        inner.replace("\\\"", "\"").replace("\\\\", "\\")
    } else {
        value.to_owned()
    }
}

fn percent_decode(encoded: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(encoded.len());
    let bytes_in = encoded.as_bytes();
    let mut index = 0;
    while index < bytes_in.len() {
        if bytes_in[index] == b'%' {
            if index + 2 >= bytes_in.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes_in[index + 1..index + 3]).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            bytes.push(bytes_in[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes).ok()
}

// Server-provided names must never escape the temporary directory.
fn sanitize_file_name(name: &str) -> Option<String> {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.bytes().any(|byte| byte < 0x20 || byte == 0x7f)
    {
        return None;
    }
    Some(truncate_name(name))
}

fn truncate_name(name: &str) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name.to_owned();
    }
    // Keep the extension so the caller's app still opens the right type.
    if let Some((stem, extension)) = name.rsplit_once('.')
        && !stem.is_empty()
        && extension.len() <= 20
    {
        let room = MAX_NAME_BYTES - extension.len() - 1;
        let mut end = room.min(stem.len());
        while !stem.is_char_boundary(end) {
            end -= 1;
        }
        if end > 0 {
            return format!("{}.{}", &stem[..end], extension);
        }
    }
    let mut end = MAX_NAME_BYTES.min(name.len());
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_owned()
}

// Completed downloads must survive long enough for the requesting app to open them.
pub(crate) fn prune_stale_downloads() {
    prune_stale_downloads_in(&std::env::temp_dir());
}

fn prune_stale_downloads_in(tmp: &Path) {
    let Ok(entries) = fs::read_dir(tmp) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let is_ours = entry.file_type().is_ok_and(|kind| kind.is_dir())
            && entry
                .file_name()
                .to_string_lossy()
                .starts_with(DOWNLOAD_PREFIX);
        if !is_ours {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .map(|modified| now.duration_since(modified).unwrap_or_default() > STALE_DOWNLOAD_AGE)
            .unwrap_or(false);
        if stale {
            let _ignored = fs::remove_dir_all(entry.path());
        }
    }
}
