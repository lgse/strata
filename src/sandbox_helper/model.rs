// SPDX-License-Identifier: MIT

use std::{fs, io::Read, path::Path};

use crate::services::{ModelFormat, ModelPreviewStage, model_preview::*};

mod embedded;
mod gcode;
mod shade;
mod stl;
mod threemf;

// The test module reaches these through `super::*`.
#[cfg(test)]
use resvg::tiny_skia::{Color, Pixmap};
use shade::Shading;
#[cfg(test)]
use stl::stl;
use threemf::package_relationships;
#[cfg(test)]
use threemf::triangles_3mf;

pub(super) fn thumbnail(
    input: &Path,
    format: ModelFormat,
    (width, height): (u32, u32),
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<Vec<u8>, String> {
    if format == ModelFormat::Gcode {
        gcode::thumbnail(input, width, height, progress)
    } else {
        embedded::thumbnail(input, format, width, height, progress)
    }
}
const _: () = assert!(
    MAX_MODEL_TRIANGLES == 2_000_000,
    "update the triangle limit message and its catalog entry"
);

// The preview pane translates this exact text, so it must not embed runtime values.
fn triangle_limit_message() -> String {
    "This model exceeds the 2 million triangle preview limit. Try a lower-detail version."
        .to_owned()
}
const MULTIPART_MODEL_MESSAGE: &str = "Multipart model detected. Unable to render preview.";
const TOO_COMPLEX_MODEL_MESSAGE: &str = "This model is too complex to draw within the preview rendering limit. Try a lower-detail version.";
const DAMAGED_MODEL_MESSAGE: &str = "This model file is invalid or damaged.";
const MODEL_INPUT_LIMIT_MESSAGE: &str =
    "This model file exceeds the 128 MiB preview limit. Try a smaller or lower-detail version.";
const VERTEX_LIMIT_MESSAGE: &str =
    "This 3MF model exceeds the 2 million vertex preview limit. Try a lower-detail version.";
const UNPACKED_LIMIT_MESSAGE: &str = "The unpacked 3MF model exceeds the 128 MiB preview limit.";
const _: () = assert!(
    MAX_MODEL_INPUT_BYTES == 128 * 1024 * 1024
        && MAX_MODEL_VERTICES == 2_000_000
        && MAX_MODEL_XML_BYTES == 128 * 1024 * 1024,
    "update the model limit messages and their catalog entries"
);

fn is_budget_failure(message: &str) -> bool {
    let exceeded = message.ends_with(" limit exceeded") || message.contains(" budget exceeded");
    let unaffordable =
        message.contains(" preview limit") || message.starts_with("Cannot allocate ");
    exceeded || unaffordable
}

/// The preview pane translates these exact texts; parse diagnostics are not actionable.
fn user_model_message(message: String) -> String {
    match message.as_str() {
        "Model exceeds the input size limit" | "Model package exceeds the input size limit" => {
            MODEL_INPUT_LIMIT_MESSAGE.to_owned()
        }
        MULTIPART_MODEL_MESSAGE
        | TOO_COMPLEX_MODEL_MESSAGE
        | VERTEX_LIMIT_MESSAGE
        | UNPACKED_LIMIT_MESSAGE
        | "STL contains no valid triangles"
        | "3MF contains no printable triangles"
        | "3MF package has no model"
        | "Model has no visible extent"
        | "External 3MF model references are not supported"
        | "This FreeCAD file has no usable embedded thumbnail"
        | "This model has no unambiguous usable embedded thumbnail"
        | NO_GCODE_THUMBNAIL => message,
        _ if message == triangle_limit_message() => message,
        _ if is_budget_failure(&message) => TOO_COMPLEX_MODEL_MESSAGE.to_owned(),
        _ => DAMAGED_MODEL_MESSAGE.to_owned(),
    }
}

type Point = [f32; 3];
type Matrix = [f32; 12];

pub(super) struct RenderedModel {
    pub(super) png: Vec<u8>,
    pub(super) mesh: Option<ModelMesh>,
}

fn rgb(value: &str) -> Result<[u8; 3], String> {
    let value = u32::from_str_radix(value, 16).map_err(|_| "Invalid model palette")?;
    Ok([(value >> 16) as u8, (value >> 8) as u8, value as u8])
}

#[cfg(test)]
fn shade(
    faces: &[[Point; 3]],
    width: u32,
    height: u32,
    accent: [u8; 3],
    surface: [u8; 4],
    view: ModelView,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<Vec<u8>, String> {
    let shading = Shading {
        width,
        height,
        accent,
        surface,
        view,
    };
    shade::shade(faces, &shading, progress)
}

fn meshed(
    faces: &[[Point; 3]],
    encoding: ModelEncoding,
    shading: &Shading,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<RenderedModel, String> {
    Ok(RenderedModel {
        png: shade::shade(faces, shading, progress)?,
        mesh: Some(ModelMesh {
            view: shading.view,
            encoding,
            triangles: faces.len(),
        }),
    })
}

fn render_3mf(
    input: &Path,
    shading: &Shading,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<RenderedModel, String> {
    let mut package = embedded::Package::open(input, ModelFormat::ThreeMf)?;
    if let Some(png) = package.thumbnail(shading.width, shading.height, progress)? {
        return Ok(RenderedModel { png, mesh: None });
    }
    if package.model_parts > 1 {
        return Err(MULTIPART_MODEL_MESSAGE.into());
    }
    progress(ModelPreviewStage::Reading);
    let faces = threemf::triangles_3mf(&package.model_xml()?)?;
    drop(package);
    meshed(&faces, ModelEncoding::ThreeMf, shading, progress)
}

fn render_stl(
    input: &Path,
    shading: &Shading,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<RenderedModel, String> {
    let mut bytes = Vec::new();
    fs::File::open(input)
        .map_err(|error| error.to_string())?
        .take(MAX_MODEL_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_MODEL_INPUT_BYTES {
        return Err("Model exceeds the input size limit".into());
    }
    let (faces, encoding) = stl::stl(&bytes)?;
    drop(bytes);
    meshed(&faces, encoding, shading, progress)
}

pub(super) fn render_reporting(
    input: &Path,
    value: &str,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<RenderedModel, String> {
    render_model(input, value, progress).map_err(user_model_message)
}

fn parse_view(view: &[&str]) -> Result<ModelView, String> {
    match view {
        [] => Ok(ModelView::default()),
        [view] => Ok(ModelView::from_argument(view).ok_or("Invalid model request")?),
        _ => Err("Invalid model request".into()),
    }
}

fn parse_size(size: &str) -> Result<(u32, u32), String> {
    let (width, height) = size.split_once('x').ok_or("Invalid model size")?;
    let dimension = |value: &str| -> Result<u32, String> {
        Ok(value
            .parse::<u32>()
            .map_err(|_| "Invalid model size")?
            .clamp(16, 800))
    };
    Ok((dimension(width)?, dimension(height)?))
}

fn parse_surface(surface: &str) -> Result<[u8; 4], String> {
    if surface == "transparent" {
        return Ok([0, 0, 0, 0]);
    }
    let [r, g, b] = rgb(surface)?;
    Ok([r, g, b, 255])
}

/// Parses `format:WxH:accent:surface[:view]`.
fn parse_request(value: &str) -> Result<(ModelFormat, Shading), String> {
    let parts: Vec<_> = value.split(':').collect();
    let [format, size, accent, surface, view @ ..] = parts.as_slice() else {
        return Err("Invalid model request".into());
    };
    let view = parse_view(view)?;
    let format = ModelFormat::from_argument(format).ok_or("Invalid model format")?;
    let (width, height) = parse_size(size)?;
    let shading = Shading {
        width,
        height,
        accent: rgb(accent)?,
        surface: parse_surface(surface)?,
        view,
    };
    Ok((format, shading))
}

fn render_model(
    input: &Path,
    value: &str,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<RenderedModel, String> {
    progress(ModelPreviewStage::Reading);
    let (format, shading) = parse_request(value)?;
    match format {
        ModelFormat::FreeCad | ModelFormat::Gcode => Ok(RenderedModel {
            png: thumbnail(input, format, (shading.width, shading.height), progress)?,
            mesh: None,
        }),
        ModelFormat::ThreeMf => render_3mf(input, &shading, progress),
        ModelFormat::Stl => render_stl(input, &shading, progress),
    }
}

#[cfg(test)]
mod tests;
