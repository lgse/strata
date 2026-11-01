// SPDX-License-Identifier: MIT

use std::{collections::HashMap, fs, io::Read, path::Path};

use crate::services::{ModelFormat, ModelPreviewStage, model_preview::*};

mod embedded;
mod gcode;

pub(super) fn thumbnail(
    input: &Path,
    format: ModelFormat,
    width: u32,
    height: u32,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<Vec<u8>, String> {
    if format == ModelFormat::Gcode {
        gcode::thumbnail(input, width, height, progress)
    } else {
        embedded::thumbnail(input, format, width, height, progress)
    }
}
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};
use resvg::tiny_skia::{Color, Pixmap};

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
        _ if message.ends_with(" limit exceeded")
            || message.contains(" budget exceeded")
            || message.contains(" preview limit")
            || message.starts_with("Cannot allocate ") =>
        {
            TOO_COMPLEX_MODEL_MESSAGE.to_owned()
        }
        _ => DAMAGED_MODEL_MESSAGE.to_owned(),
    }
}

type Point = [f32; 3];
type Matrix = [f32; 12];

pub(super) struct RenderedModel {
    pub(super) png: Vec<u8>,
    pub(super) mesh: Option<ModelMesh>,
}
const IDENTITY: Matrix = [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.];

#[derive(Default)]
struct Object {
    vertices: Vec<Point>,
    triangles: Vec<[usize; 3]>,
    components: Vec<(u32, Matrix)>,
}

fn attribute(tag: &BytesStart<'_>, name: &[u8]) -> Result<Option<String>, String> {
    for attr in tag.attributes() {
        let attr = attr.map_err(|_| "Invalid 3MF attribute")?;
        if attr.key.local_name().as_ref() == name {
            return String::from_utf8(attr.value.into_owned())
                .map(Some)
                .map_err(|_| "Invalid 3MF attribute".into());
        }
    }
    Ok(None)
}

fn number(tag: &BytesStart<'_>, name: &[u8]) -> Result<f32, String> {
    let value = attribute(tag, name)?.ok_or("Missing 3MF coordinate")?;
    let number: f32 = value.parse().map_err(|_| "Invalid 3MF coordinate")?;
    number
        .is_finite()
        .then_some(number)
        .ok_or("Invalid 3MF coordinate".into())
}

fn index(tag: &BytesStart<'_>, name: &[u8]) -> Result<u32, String> {
    attribute(tag, name)?
        .ok_or("Missing 3MF index")?
        .parse()
        .map_err(|_| "Invalid 3MF index".into())
}

fn matrix(value: Option<String>) -> Result<Matrix, String> {
    let Some(value) = value else {
        return Ok(IDENTITY);
    };
    let parts: Vec<f32> = value
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| "Invalid 3MF transform")?;
    if parts.len() != 12 || parts.iter().any(|part| !part.is_finite()) {
        return Err("Invalid 3MF transform".into());
    }
    parts.try_into().map_err(|_| "Invalid 3MF transform".into())
}

fn transform(m: Matrix, p: Point) -> Point {
    [
        m[0] * p[0] + m[3] * p[1] + m[6] * p[2] + m[9],
        m[1] * p[0] + m[4] * p[1] + m[7] * p[2] + m[10],
        m[2] * p[0] + m[5] * p[1] + m[8] * p[2] + m[11],
    ]
}

fn combine(a: Matrix, b: Matrix) -> Matrix {
    let origin = transform(a, transform(b, [0., 0., 0.]));
    let mut result = [0.; 12];
    for axis in 0..3 {
        let mut basis = [0.; 3];
        basis[axis] = 1.;
        let point = transform(a, transform(b, basis));
        for component in 0..3 {
            result[axis * 3 + component] = point[component] - origin[component];
        }
    }
    result[9..].copy_from_slice(&origin);
    result
}

fn triangles_3mf(xml: &[u8]) -> Result<Vec<[Point; 3]>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut objects = HashMap::<u32, Object>::new();
    let mut current = None;
    let mut items = Vec::new();
    let mut vertices = 0;
    let mut triangles = 0;
    let mut components = 0;
    loop {
        let event = reader.read_event().map_err(|_| "Invalid 3MF model XML")?;
        match event {
            Event::Start(tag) | Event::Empty(tag) => match tag.local_name().as_ref() {
                b"object" => {
                    let id = index(&tag, b"id")?;
                    if objects.len() >= MAX_3MF_OBJECTS
                        || objects.insert(id, Object::default()).is_some()
                    {
                        return Err("3MF object limit exceeded".into());
                    }
                    current = Some(id);
                }
                b"vertex" if current.is_some() => {
                    vertices += 1;
                    if vertices > MAX_MODEL_VERTICES {
                        return Err(VERTEX_LIMIT_MESSAGE.to_owned());
                    }
                    let point = [
                        number(&tag, b"x")?,
                        number(&tag, b"y")?,
                        number(&tag, b"z")?,
                    ];
                    objects
                        .get_mut(&current.unwrap_or_default())
                        .ok_or("Invalid 3MF object")?
                        .vertices
                        .push(point);
                }
                b"triangle" if current.is_some() => {
                    triangles += 1;
                    if triangles > MAX_MODEL_TRIANGLES {
                        return Err(triangle_limit_message());
                    }
                    let indices = [
                        index(&tag, b"v1")? as usize,
                        index(&tag, b"v2")? as usize,
                        index(&tag, b"v3")? as usize,
                    ];
                    objects
                        .get_mut(&current.unwrap_or_default())
                        .ok_or("Invalid 3MF object")?
                        .triangles
                        .push(indices);
                }
                b"component" if current.is_some() => {
                    components += 1;
                    if components > MAX_MODEL_COMPONENT_REFERENCES {
                        return Err("3MF component reference limit exceeded".into());
                    }
                    if attribute(&tag, b"path")?.is_some() {
                        return Err(MULTIPART_MODEL_MESSAGE.into());
                    }
                    let id = index(&tag, b"objectid")?;
                    let matrix = matrix(attribute(&tag, b"transform")?)?;
                    objects
                        .get_mut(&current.unwrap_or_default())
                        .ok_or("Invalid 3MF object")?
                        .components
                        .push((id, matrix));
                }
                b"item" => {
                    if attribute(&tag, b"path")?.is_some() {
                        return Err(MULTIPART_MODEL_MESSAGE.into());
                    }
                    if items.len() >= MAX_3MF_BUILD_ITEMS {
                        return Err("3MF build item limit exceeded".into());
                    }
                    items.push((
                        index(&tag, b"objectid")?,
                        matrix(attribute(&tag, b"transform")?)?,
                    ));
                }
                _ => {}
            },
            Event::End(tag) if tag.local_name().as_ref() == b"object" => current = None,
            Event::Eof => break,
            _ => {}
        }
    }
    let mut faces = Vec::new();
    let mut stack: Vec<_> = items.into_iter().map(|(id, m)| (id, m, 0)).collect();
    let mut expanded = 0;
    while let Some((id, matrix, depth)) = stack.pop() {
        expanded += 1;
        if expanded > MAX_MODEL_COMPONENT_EXPANSIONS {
            return Err("3MF component expansion limit exceeded".into());
        }
        if depth > MAX_3MF_COMPONENT_DEPTH {
            return Err("3MF component nesting limit exceeded".into());
        }
        let object = objects.get(&id).ok_or("3MF references a missing object")?;
        if expanded + stack.len() + object.components.len() > MAX_MODEL_COMPONENT_EXPANSIONS {
            return Err("3MF component expansion limit exceeded".into());
        }
        for &(id, child) in &object.components {
            stack.push((id, combine(matrix, child), depth + 1));
        }
        for indices in &object.triangles {
            let [Some(a), Some(b), Some(c)] =
                indices.map(|index| object.vertices.get(index).copied())
            else {
                return Err("3MF triangle references a missing vertex".into());
            };
            if faces.len() >= MAX_MODEL_TRIANGLES {
                return Err(triangle_limit_message());
            }
            faces.push([
                transform(matrix, a),
                transform(matrix, b),
                transform(matrix, c),
            ]);
        }
    }
    if faces.is_empty() {
        return Err("3MF contains no printable triangles".into());
    }
    Ok(faces)
}

fn stl(bytes: &[u8]) -> Result<(Vec<[Point; 3]>, ModelEncoding), String> {
    if bytes.len() >= 84 {
        let count =
            u32::from_le_bytes(bytes[80..84].try_into().map_err(|_| "Invalid STL")?) as usize;
        if count.checked_mul(50).and_then(|n| n.checked_add(84)) == Some(bytes.len()) {
            if count > MAX_MODEL_TRIANGLES {
                return Err(triangle_limit_message());
            }
            let mut faces = Vec::with_capacity(count);
            for face in bytes[84..].as_chunks::<50>().0 {
                let mut points = [[0.; 3]; 3];
                for (vertex, point) in points.iter_mut().enumerate() {
                    for (axis, coordinate) in point.iter_mut().enumerate() {
                        let start = 12 + vertex * 12 + axis * 4;
                        *coordinate = f32::from_le_bytes(
                            face[start..start + 4]
                                .try_into()
                                .map_err(|_| "Invalid STL")?,
                        );
                        if !coordinate.is_finite() {
                            return Err("Invalid STL coordinate".into());
                        }
                    }
                }
                faces.push(points);
            }
            return Ok((faces, ModelEncoding::BinaryStl));
        }
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "Invalid STL")?;
    if !text.trim_start().starts_with("solid") {
        return Err("Invalid STL".into());
    }
    let mut faces = Vec::new();
    let mut points = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() != Some("vertex") {
            continue;
        }
        let coordinate = |field: Option<&str>| -> Result<f32, String> {
            let value: f32 = field
                .ok_or("Invalid STL vertex")?
                .parse()
                .map_err(|_| "Invalid STL vertex")?;
            value
                .is_finite()
                .then_some(value)
                .ok_or("Invalid STL coordinate".into())
        };
        let point = [
            coordinate(fields.next())?,
            coordinate(fields.next())?,
            coordinate(fields.next())?,
        ];
        points.push(point);
        if points.len() == 3 {
            if faces.len() >= MAX_MODEL_TRIANGLES {
                return Err(triangle_limit_message());
            }
            faces.push([points[0], points[1], points[2]]);
            points.clear();
        }
    }
    if !points.is_empty() || faces.is_empty() {
        return Err("STL contains no valid triangles".into());
    }
    Ok((faces, ModelEncoding::AsciiStl))
}

fn rgb(value: &str) -> Result<[u8; 3], String> {
    let value = u32::from_str_radix(value, 16).map_err(|_| "Invalid model palette")?;
    Ok([(value >> 16) as u8, (value >> 8) as u8, value as u8])
}

fn shade(
    faces: &[[Point; 3]],
    width: u32,
    height: u32,
    accent: [u8; 3],
    surface: [u8; 4],
    view: ModelView,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<Vec<u8>, String> {
    // Keep empty canvas from separating the model and its centred player controls.
    let side = width.min(height);
    let (width, height) = (side, side);
    progress(ModelPreviewStage::Rendering {
        triangles: faces.len(),
    });
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for face in faces {
        for p in face {
            for axis in 0..3 {
                if !p[axis].is_finite() {
                    return Err("Invalid transformed model coordinate".into());
                }
                min[axis] = min[axis].min(p[axis]);
                max[axis] = max[axis].max(p[axis]);
            }
        }
    }
    let span: Point = std::array::from_fn(|axis| max[axis] - min[axis]);
    let center: Point = std::array::from_fn(|axis| min[axis] + span[axis] * 0.5);
    let root_six = 6_f32.sqrt();
    // Fit the same bounding box for all four cameras so switching never zooms.
    let extent = ((span[0] + span[1]) * std::f32::consts::FRAC_1_SQRT_2)
        .max((span[0] + span[1] + 2. * span[2]) / root_six);
    if !extent.is_finite() || extent <= 0. {
        return Err("Model has no visible extent".into());
    }
    let scale = (width.min(height) as f32 * 0.82) / extent;
    let [sx, sy, _] = view.camera_direction();
    let project = |face: [Point; 3]| {
        face.map(|p| {
            let [x, y, z] = std::array::from_fn(|axis| p[axis] - center[axis]);
            [
                (-sy * x + sx * y) * std::f32::consts::FRAC_1_SQRT_2,
                (-sx * x - sy * y + 2. * z) / root_six,
                (sx * x + sy * y + z) / 3_f32.sqrt(),
            ]
        })
    };
    let mut pixmap = Pixmap::new(width, height).ok_or("Cannot allocate model preview")?;
    pixmap.fill(Color::from_rgba8(
        surface[0], surface[1], surface[2], surface[3],
    ));
    let mut depths = vec![f32::NEG_INFINITY; (width * height) as usize];
    let mut work = 0u64;
    for face in faces {
        let points = project(*face);
        let a = [
            points[1][0] - points[0][0],
            points[1][1] - points[0][1],
            points[1][2] - points[0][2],
        ];
        let b = [
            points[2][0] - points[0][0],
            points[2][1] - points[0][1],
            points[2][2] - points[0][2],
        ];
        let normal = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let length = (normal.iter().map(|n| n * n).sum::<f32>()).sqrt();
        let light = if length > 0. {
            (0.58 + 0.36 * (normal[0] * 0.3 - normal[1] * 0.5 + normal[2] * 0.8).abs() / length)
                .clamp(0.35, 0.94)
        } else {
            0.58
        };
        let face = points.map(|point| {
            [
                width as f32 * 0.5 + point[0] * scale,
                height as f32 * 0.5 - point[1] * scale,
                point[2],
            ]
        });
        let edge = |a: Point, b: Point, p: Point| {
            (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
        };
        let area = edge(face[0], face[1], face[2]);
        if area.abs() < 1e-8 {
            continue;
        }
        let color = [
            (accent[0] as f32 * light + surface[0] as f32 * (1. - light) * 0.35) as u8,
            (accent[1] as f32 * light + surface[1] as f32 * (1. - light) * 0.35) as u8,
            (accent[2] as f32 * light + surface[2] as f32 * (1. - light) * 0.35) as u8,
            255,
        ];
        let lower = |axis| {
            face.iter()
                .map(|p| p[axis])
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.) as u32
        };
        let upper = |axis, bound| {
            face.iter()
                .map(|p| p[axis])
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil()
                .max(0.)
                .min(bound as f32) as u32
        };
        let (left, right, top, bottom) = (lower(0), upper(0, width), lower(1), upper(1, height));
        work += u64::from(right.saturating_sub(left)) * u64::from(bottom.saturating_sub(top));
        if work > MAX_MODEL_RASTER_WORK {
            return Err(TOO_COMPLEX_MODEL_MESSAGE.into());
        }
        for y in top..bottom {
            for x in left..right {
                let point = [x as f32 + 0.5, y as f32 + 0.5, 0.];
                let weights = [
                    edge(face[1], face[2], point) / area,
                    edge(face[2], face[0], point) / area,
                    edge(face[0], face[1], point) / area,
                ];
                if weights.iter().any(|weight| *weight < 0.) {
                    continue;
                }
                let depth = weights
                    .iter()
                    .zip(face)
                    .map(|(weight, point)| weight * point[2])
                    .sum();
                let index = (y * width + x) as usize;
                if depth > depths[index] {
                    depths[index] = depth;
                    pixmap.data_mut()[index * 4..index * 4 + 4].copy_from_slice(&color);
                }
            }
        }
    }
    progress(ModelPreviewStage::Finishing);
    pixmap.encode_png().map_err(|error| error.to_string())
}

fn package_relationships(
    archive: &mut zip::ZipArchive<fs::File>,
) -> Result<(Option<String>, Vec<String>), String> {
    let file = match archive.by_name("_rels/.rels") {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) => return Ok((None, Vec::new())),
        Err(_) => return Err("Unable to read 3MF package relationships".into()),
    };
    let mut xml = Vec::new();
    file.take(MAX_3MF_RELATIONSHIPS_BYTES + 1)
        .read_to_end(&mut xml)
        .map_err(|_| "Invalid 3MF package relationships")?;
    if xml.len() as u64 > MAX_3MF_RELATIONSHIPS_BYTES {
        return Err(format!(
            "3MF package relationships exceed the {} KiB preview limit",
            MAX_3MF_RELATIONSHIPS_BYTES / 1024
        ));
    }
    let mut reader = Reader::from_reader(xml.as_slice());
    let mut model = None;
    let mut thumbnails = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|_| "Invalid 3MF package relationships")?
        {
            Event::Start(tag) | Event::Empty(tag)
                if tag.local_name().as_ref() == b"Relationship" =>
            {
                let kind = attribute(&tag, b"Type")?.unwrap_or_default();
                let is_model =
                    kind == "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";
                let is_thumbnail = kind
                    == "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail";
                if !is_model && !is_thumbnail {
                    continue;
                }
                if attribute(&tag, b"TargetMode")?.as_deref() == Some("External") {
                    if is_model {
                        return Err("External 3MF model references are not supported".into());
                    }
                    continue;
                }
                let target =
                    attribute(&tag, b"Target")?.ok_or("Missing 3MF relationship target")?;
                let target = quick_xml::escape::unescape(&target)
                    .map_err(|_| "Invalid 3MF relationship target")?;
                let target = target.trim_start_matches('/');
                if target.is_empty()
                    || target.contains(['\\', ':', '?', '#'])
                    || target
                        .split('/')
                        .any(|part| matches!(part, ".." | "." | ""))
                {
                    return Err("Invalid 3MF package part reference".into());
                }
                if is_model {
                    if model.replace(target.to_owned()).is_some() {
                        return Err("3MF package contains multiple root model relationships".into());
                    }
                } else {
                    thumbnails.push(target.to_owned());
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if model.is_none() {
        return Err("3MF package has no root model relationship".into());
    }
    Ok((model, thumbnails))
}

fn render_3mf(
    input: &Path,
    width: u32,
    height: u32,
    accent: [u8; 3],
    surface: [u8; 4],
    view: ModelView,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<RenderedModel, String> {
    let mut package = embedded::Package::open(input, ModelFormat::ThreeMf)?;
    if let Some(png) = package.thumbnail(width, height, progress)? {
        return Ok(RenderedModel { png, mesh: None });
    }
    if package.model_parts > 1 {
        return Err(MULTIPART_MODEL_MESSAGE.into());
    }
    progress(ModelPreviewStage::Reading);
    let faces = triangles_3mf(&package.model_xml()?)?;
    drop(package);
    Ok(RenderedModel {
        png: shade(&faces, width, height, accent, surface, view, progress)?,
        mesh: Some(ModelMesh {
            view,
            encoding: ModelEncoding::ThreeMf,
            triangles: faces.len(),
        }),
    })
}

pub(super) fn render_reporting(
    input: &Path,
    value: &str,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<RenderedModel, String> {
    render_model(input, value, progress).map_err(user_model_message)
}

fn render_model(
    input: &Path,
    value: &str,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<RenderedModel, String> {
    progress(ModelPreviewStage::Reading);
    let parts: Vec<_> = value.split(':').collect();
    let [format, size, accent, surface, view @ ..] = parts.as_slice() else {
        return Err("Invalid model request".into());
    };
    let view = match view {
        [] => ModelView::default(),
        [view] => ModelView::from_argument(view).ok_or("Invalid model request")?,
        _ => return Err("Invalid model request".into()),
    };
    let format = ModelFormat::from_argument(format).ok_or("Invalid model format")?;
    let (width, height) = size.split_once('x').ok_or("Invalid model size")?;
    let width = width
        .parse::<u32>()
        .map_err(|_| "Invalid model size")?
        .clamp(16, 800);
    let height = height
        .parse::<u32>()
        .map_err(|_| "Invalid model size")?
        .clamp(16, 800);
    let accent = rgb(accent)?;
    let surface = if *surface == "transparent" {
        [0, 0, 0, 0]
    } else {
        let [r, g, b] = rgb(surface)?;
        [r, g, b, 255]
    };
    match format {
        ModelFormat::FreeCad | ModelFormat::Gcode => Ok(RenderedModel {
            png: thumbnail(input, format, width, height, progress)?,
            mesh: None,
        }),
        ModelFormat::ThreeMf => render_3mf(input, width, height, accent, surface, view, progress),
        ModelFormat::Stl => {
            let mut bytes = Vec::new();
            fs::File::open(input)
                .map_err(|error| error.to_string())?
                .take(MAX_MODEL_INPUT_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            if bytes.len() as u64 > MAX_MODEL_INPUT_BYTES {
                return Err("Model exceeds the input size limit".into());
            }
            let (faces, encoding) = stl(&bytes)?;
            drop(bytes);
            Ok(RenderedModel {
                png: shade(&faces, width, height, accent, surface, view, progress)?,
                mesh: Some(ModelMesh {
                    view,
                    encoding,
                    triangles: faces.len(),
                }),
            })
        }
    }
}

#[cfg(test)]
mod tests;
