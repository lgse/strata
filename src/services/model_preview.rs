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
/// Text G-code is scanned until the first command, at most this far; a
/// thumbnail block that has already started may run past it.
pub(crate) const MAX_GCODE_HEADER_BYTES: u64 = 2 * 1024 * 1024;
pub(crate) const NO_GCODE_THUMBNAIL: &str = "This G-code file has no usable embedded thumbnail";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ModelFormat {
    Stl,
    ThreeMf,
    FreeCad,
    Gcode,
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
            "gcode" | "gco" | "bgcode" => Some(Self::Gcode),
            _ => None,
        }
    }

    pub(crate) fn argument(self) -> &'static str {
        match self {
            Self::Stl => "stl",
            Self::ThreeMf => "3mf",
            Self::FreeCad => "fcstd",
            Self::Gcode => "gcode",
        }
    }

    pub(crate) fn from_argument(value: &str) -> Option<Self> {
        match value {
            "stl" => Some(Self::Stl),
            "3mf" => Some(Self::ThreeMf),
            "fcstd" => Some(Self::FreeCad),
            "gcode" => Some(Self::Gcode),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ModelPalette {
    pub accent: u32,
    pub surface: u32,
}

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize,
)]
pub enum ModelView {
    #[default]
    FrontLeft,
    FrontRight,
    BackRight,
    BackLeft,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ModelEncoding {
    BinaryStl,
    AsciiStl,
    ThreeMf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelMesh {
    pub view: ModelView,
    pub encoding: ModelEncoding,
    pub triangles: usize,
}

impl ModelView {
    pub(crate) const ALL: [Self; 4] = [
        Self::FrontLeft,
        Self::FrontRight,
        Self::BackRight,
        Self::BackLeft,
    ];

    pub(crate) fn index(self) -> usize {
        match self {
            Self::FrontLeft => 0,
            Self::FrontRight => 1,
            Self::BackRight => 2,
            Self::BackLeft => 3,
        }
    }

    /// From the model centre toward the camera: +X is right, −Y front and +Z up.
    pub(crate) fn camera_direction(self) -> [f32; 3] {
        match self {
            Self::FrontLeft => [-1., -1., 1.],
            Self::FrontRight => [1., -1., 1.],
            Self::BackRight => [1., 1., 1.],
            Self::BackLeft => [-1., 1., 1.],
        }
    }

    pub(crate) fn step(self, step: i32) -> Self {
        Self::ALL[(self.index() as i32 - step.rem_euclid(4)).rem_euclid(4) as usize]
    }

    pub(crate) fn argument(self) -> &'static str {
        match self {
            Self::FrontLeft => "front-left",
            Self::FrontRight => "front-right",
            Self::BackRight => "back-right",
            Self::BackLeft => "back-left",
        }
    }

    pub(crate) fn from_argument(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.argument() == value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ModelRenderPurpose {
    Preview,
    Thumbnail,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ModelRender {
    pub purpose: ModelRenderPurpose,
    pub format: ModelFormat,
    pub size: MediaPreviewSize,
    pub palette: ModelPalette,
    pub view: ModelView,
}
