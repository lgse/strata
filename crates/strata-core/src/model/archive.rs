// SPDX-License-Identifier: MIT

use super::FileEntry;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveFormat {
    Zip,
    SevenZ,
    TarGz,
    Tar,
    Rar,
}

impl ArchiveFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::SevenZ => "7z",
            Self::TarGz => "tar.gz",
            Self::Tar => "tar",
            Self::Rar => "rar",
        }
    }

    pub fn supports_password(self) -> bool {
        matches!(self, Self::Zip | Self::SevenZ | Self::Rar)
    }

    pub fn from_extension(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            Some(Self::TarGz)
        } else if lower.ends_with(".tar") {
            Some(Self::Tar)
        } else if lower.ends_with(".zip") {
            Some(Self::Zip)
        } else if lower.ends_with(".7z") {
            Some(Self::SevenZ)
        } else if cfg!(feature = "rar") && lower.ends_with(".rar") {
            Some(Self::Rar)
        } else {
            None
        }
    }

    pub fn for_entry(entry: &FileEntry) -> Option<Self> {
        if !entry.is_file() || entry.location.native_path().is_none() {
            return None;
        }
        Self::from_extension(&entry.display_name)
    }
}
