// SPDX-License-Identifier: MIT

use glib::{KeyFile, KeyFileFlags, Uri, UriFlags};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WebLink {
    pub(crate) address: String,
    pub(crate) name: String,
}

impl WebLink {
    pub(crate) fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        if input.len() > 16 * 1024 || input.chars().any(char::is_whitespace) {
            return None;
        }
        let uri = Uri::parse(input, UriFlags::ENCODED).ok()?;
        if !matches!(uri.scheme().to_ascii_lowercase().as_str(), "http" | "https")
            || uri.userinfo().is_some()
            || input.chars().any(char::is_control)
        {
            return None;
        }
        let host = uri.host().filter(|host| !host.is_empty())?;
        let name: String = host
            .chars()
            .take(100)
            .map(|c| {
                if matches!(c, '/' | '\\' | ':') {
                    '-'
                } else {
                    c
                }
            })
            .collect();
        Some(Self {
            address: input.to_owned(),
            name,
        })
    }

    pub(crate) fn desktop_entry(&self) -> Vec<u8> {
        let entry = KeyFile::new();
        entry.set_string("Desktop Entry", "Type", "Link");
        entry.set_string("Desktop Entry", "Name", &self.name);
        entry.set_string("Desktop Entry", "URL", &self.address);
        entry.set_string("Desktop Entry", "Icon", "text-html");
        entry.to_data().as_bytes().to_vec()
    }

    /// Application launchers remain on the normal opening path; link files never run Exec.
    pub(crate) fn from_desktop_entry(input: &str) -> Result<Option<Self>, ()> {
        let entry = KeyFile::new();
        entry
            .load_from_data(input, KeyFileFlags::NONE)
            .map_err(|_| ())?;
        if entry.string("Desktop Entry", "Type").as_deref() != Ok("Link") {
            return Ok(None);
        }
        if entry.has_key("Desktop Entry", "Exec").unwrap_or(false) {
            return Err(());
        }
        let address = entry.string("Desktop Entry", "URL").map_err(|_| ())?;
        Self::parse(&address).map(Some).ok_or(())
    }
}
