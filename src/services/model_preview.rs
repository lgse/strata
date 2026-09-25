// SPDX-License-Identifier: MIT

use std::{ffi::OsStr, path::Path};

use super::MediaPreviewSize;

// Input and geometry limits are independent; neither is a resident-memory budget.
pub(crate) const MAX_MODEL_INPUT_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const MAX_MODEL_XML_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const MAX_3MF_ARCHIVE_ENTRIES: usize = 256;
pub(crate) const MAX_FREECAD_ARCHIVE_ENTRIES: usize = 4096;
pub(crate) const MAX_3MF_OBJECTS: usize = 1024;
pub(crate) const MAX_3MF_BUILD_ITEMS: usize = 1024;
pub(crate) const MAX_3MF_COMPONENT_DEPTH: usize = 16;
pub(crate) const MAX_3MF_RELATIONSHIPS_BYTES: u64 = 64 * 1024;
pub(crate) const MAX_MODEL_TRIANGLES: usize = 2_000_000;
pub(crate) const MAX_MODEL_VERTICES: usize = 2_000_000;
pub(crate) const MAX_MODEL_COMPONENT_REFERENCES: usize = 100_000;
pub(crate) const MAX_MODEL_COMPONENT_EXPANSIONS: usize = 100_000;
pub(crate) const MAX_MODEL_RASTER_WORK: u64 = 100_000_000;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ModelFormat {
    Stl,
    ThreeMf,
    FreeCad,
}

impl ModelFormat {
    pub(crate) fn for_name(name: &OsStr) -> Option<Self> {
        match Path::new(name)
            .extension()?
            .to_str()?
            .to_ascii_lowercase()
            .as_str()
        {
            "stl" => Some(Self::Stl),
            "3mf" => Some(Self::ThreeMf),
            "fcstd" => Some(Self::FreeCad),
            _ => None,
        }
    }

    pub(crate) fn argument(self) -> &'static str {
        match self {
            Self::Stl => "stl",
            Self::ThreeMf => "3mf",
            Self::FreeCad => "fcstd",
        }
    }

    pub(crate) fn from_argument(value: &str) -> Option<Self> {
        match value {
            "stl" => Some(Self::Stl),
            "3mf" => Some(Self::ThreeMf),
            "fcstd" => Some(Self::FreeCad),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ModelPalette {
    pub accent: u32,
    pub surface: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ModelRender {
    pub format: ModelFormat,
    pub size: MediaPreviewSize,
    pub palette: ModelPalette,
}
