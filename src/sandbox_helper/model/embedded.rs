// SPDX-License-Identifier: MIT

use std::{fs, io::Read, path::Path};

use resvg::tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};

use crate::services::{
    ModelFormat, ModelPreviewStage,
    model_preview::{MAX_3MF_ARCHIVE_ENTRIES, MAX_FREECAD_ARCHIVE_ENTRIES},
};

const MAX_THUMBNAIL_BYTES: u64 = 4 * 1024 * 1024;
const MAX_THUMBNAIL_CANDIDATES: usize = 16;
const MAX_TOTAL_THUMBNAIL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TOTAL_THUMBNAIL_PIXELS: u64 = 16 * 1024 * 1024;

pub(super) struct Package {
    archive: zip::ZipArchive<fs::File>,
    pub(super) model: Option<usize>,
    pub(super) model_parts: usize,
    thumbnails: Vec<usize>,
}

impl Package {
    pub(super) fn open(input: &Path, format: ModelFormat) -> Result<Self, String> {
        let file = fs::File::open(input).map_err(|error| error.to_string())?;
        if file.metadata().map_err(|error| error.to_string())?.len()
            > crate::services::model_preview::MAX_MODEL_INPUT_BYTES
        {
            return Err("Model package exceeds the input size limit".into());
        }
        let mut archive = zip::ZipArchive::new(file).map_err(|_| "Invalid model package")?;
        let entry_limit = match format {
            ModelFormat::ThreeMf => MAX_3MF_ARCHIVE_ENTRIES,
            ModelFormat::FreeCad => MAX_FREECAD_ARCHIVE_ENTRIES,
            ModelFormat::Stl => return Err("STL has no embedded thumbnail".into()),
        };
        if archive.len() > entry_limit {
            return Err("Model package entry limit exceeded".into());
        }
        let (root, thumbnail_parts) = if format == ModelFormat::ThreeMf {
            super::package_relationships(&mut archive)?
        } else {
            (None, Vec::new())
        };
        let mut model = None;
        let mut model_parts = 0;
        let mut thumbnails = Vec::new();
        for i in 0..archive.len() {
            let file = archive
                .by_index_raw(i)
                .map_err(|_| "Invalid model package")?;
            let name = file.name().to_ascii_lowercase();
            if format == ModelFormat::ThreeMf {
                if name.ends_with(".model") {
                    model_parts += 1;
                }
                if root
                    .as_ref()
                    .map_or(name == "3d/3dmodel.model", |root| file.name() == root)
                {
                    model = Some(i);
                }
                if name.ends_with("thumbnail.png")
                    || thumbnail_parts.iter().any(|part| part == file.name())
                {
                    thumbnails.push(i);
                }
            } else if file.name() == "thumbnails/Thumbnail.png" {
                thumbnails.push(i);
            }
        }
        if thumbnails.len() > MAX_THUMBNAIL_CANDIDATES {
            return Err("Model thumbnail candidate limit exceeded".into());
        }
        Ok(Self {
            archive,
            model,
            model_parts,
            thumbnails,
        })
    }

    pub(super) fn thumbnail(
        &mut self,
        width: u32,
        height: u32,
        progress: &dyn Fn(ModelPreviewStage),
    ) -> Result<Option<Vec<u8>>, String> {
        if !self.thumbnails.is_empty() {
            progress(ModelPreviewStage::Thumbnail);
        }
        let mut selected = None;
        let mut remaining_bytes = MAX_TOTAL_THUMBNAIL_BYTES;
        let mut remaining_pixels = MAX_TOTAL_THUMBNAIL_PIXELS;
        for &i in &self.thumbnails {
            let Ok(file) = self.archive.by_index(i) else {
                continue;
            };
            if file.size() > MAX_THUMBNAIL_BYTES {
                continue;
            }
            let limit = MAX_THUMBNAIL_BYTES.min(remaining_bytes);
            let mut bytes = Vec::new();
            let decoded = file.take(limit + 1).read_to_end(&mut bytes).is_ok();
            if bytes.len() as u64 > remaining_bytes {
                return Err("Model thumbnail byte budget exceeded".into());
            }
            remaining_bytes -= bytes.len() as u64;
            if !decoded || bytes.len() as u64 > MAX_THUMBNAIL_BYTES {
                continue;
            }
            let Some((w, h)) = crate::sandbox::png_dimensions(&bytes) else {
                continue;
            };
            let pixels = u64::from(w) * u64::from(h);
            if pixels > MAX_TOTAL_THUMBNAIL_PIXELS {
                continue;
            }
            remaining_pixels = remaining_pixels
                .checked_sub(pixels)
                .ok_or("Model thumbnail pixel budget exceeded")?;
            if let Ok(png) = thumbnail_png(&bytes, width, height) {
                if selected.is_some() {
                    return Ok(None);
                }
                selected = Some(png);
            }
        }
        if selected.is_some() {
            progress(ModelPreviewStage::Finishing);
        }
        Ok(selected)
    }

    pub(super) fn model_xml(&mut self) -> Result<Vec<u8>, String> {
        use crate::services::model_preview::MAX_MODEL_XML_BYTES;
        let i = self.model.ok_or("3MF package has no model")?;
        let file = self.archive.by_index(i).map_err(|_| "Invalid 3MF model")?;
        let limit_message = || {
            format!(
                "The unpacked 3MF model exceeds the {} MiB preview limit.",
                MAX_MODEL_XML_BYTES / (1024 * 1024)
            )
        };
        if file.size() > MAX_MODEL_XML_BYTES {
            return Err(limit_message());
        }
        let mut xml = Vec::new();
        file.take(MAX_MODEL_XML_BYTES + 1)
            .read_to_end(&mut xml)
            .map_err(|_| "Invalid 3MF model")?;
        if xml.len() as u64 > MAX_MODEL_XML_BYTES {
            return Err(limit_message());
        }
        Ok(xml)
    }
}

pub(in crate::sandbox_helper) fn thumbnail(
    input: &Path,
    format: ModelFormat,
    width: u32,
    height: u32,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<Vec<u8>, String> {
    Package::open(input, format)?
        .thumbnail(width, height, progress)?
        .ok_or_else(|| match format {
            ModelFormat::FreeCad => "This FreeCAD file has no usable embedded thumbnail".into(),
            _ => "This model has no unambiguous usable embedded thumbnail".into(),
        })
}

#[cfg(test)]
mod tests;

fn thumbnail_png(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    // Decode in this job: toolkit loaders may try to launch another sandbox.
    let image = Pixmap::decode_png(bytes).map_err(|_| "Invalid model thumbnail")?;
    let scale = (width as f32 / image.width() as f32)
        .min(height as f32 / image.height() as f32)
        .min(1.);
    if scale >= 1. {
        return image
            .encode_png()
            .map_err(|_| "Invalid model thumbnail".into());
    }
    let mut output = Pixmap::new(
        (image.width() as f32 * scale).round().max(1.) as u32,
        (image.height() as f32 * scale).round().max(1.) as u32,
    )
    .ok_or("Cannot allocate model thumbnail")?;
    output.draw_pixmap(
        0,
        0,
        image.as_ref(),
        &PixmapPaint {
            quality: FilterQuality::Bilinear,
            ..PixmapPaint::default()
        },
        Transform::from_scale(scale, scale),
        None,
    );
    output
        .encode_png()
        .map_err(|_| "Invalid model thumbnail".into())
}
