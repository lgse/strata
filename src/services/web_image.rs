// SPDX-License-Identifier: MIT

use std::{io, path::Path, time::Duration};

use glib::{Uri, UriFlags};
use gtk::{gdk, gio};

use super::WebLink;

#[cfg(test)]
mod tests;

pub(crate) const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

pub(crate) struct WebImage {
    pub(crate) extension: &'static str,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) fn image_bytes(bytes: Vec<u8>) -> io::Result<WebImage> {
    let (kind, _) = gio::content_type_guess(None::<&Path>, Some(bytes.as_slice()));
    let mime = gio::content_type_get_mime_type(&kind).unwrap_or_default();
    let extension = match mime.as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        "image/bmp" | "image/x-bmp" | "image/x-ms-bmp" => "bmp",
        "image/tiff" => "tiff",
        "image/vnd.microsoft.icon" | "image/x-icon" => "ico",
        _ => return Err(io::Error::from(io::ErrorKind::InvalidData)),
    };
    gdk::Texture::from_bytes(&glib::Bytes::from(bytes.as_slice()))
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    Ok(WebImage { extension, bytes })
}

pub(crate) fn download_address(link: &WebLink) -> String {
    let uri = Uri::parse(&link.address, UriFlags::ENCODED).expect("validated URL");
    if uri
        .host()
        .is_some_and(|host| host.eq_ignore_ascii_case("github.com"))
    {
        let path = uri.path();
        let segments: Vec<_> = path.trim_start_matches('/').split('/').collect();
        if segments.len() >= 5 && segments[2] == "blob" {
            return format!(
                "https://raw.githubusercontent.com/{}/{}/{}",
                segments[0],
                segments[1],
                segments[3..].join("/")
            );
        }
    }
    link.address
        .split('#')
        .next()
        .unwrap_or(&link.address)
        .to_owned()
}

pub(crate) fn download(link: &WebLink) -> io::Result<Option<WebImage>> {
    let config = ureq::Agent::config_builder()
        // Redirects must not turn a browser drop into a request to host-only services.
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_recv_response(Some(Duration::from_secs(8)))
        .timeout_recv_body(Some(Duration::from_secs(30)))
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = match agent
        .get(download_address(link))
        .header("User-Agent", "strata-file-manager")
        .header("Accept", "image/png,image/jpeg,image/webp,*/*;q=0.5")
        .call()
    {
        Ok(response) => response,
        // An unreachable URL can still be saved as a link, including offline.
        Err(_) => return Ok(None),
    };
    let mime = response
        .headers()
        .get("Content-Type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if response.status().is_redirection() || !mime.starts_with("image/") {
        return Ok(None);
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_IMAGE_BYTES as u64)
        .read_to_vec()
        .map_err(io::Error::other)?;
    image_bytes(bytes).map(Some)
}
