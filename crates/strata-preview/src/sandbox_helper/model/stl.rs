// SPDX-License-Identifier: MIT
//! Binary and ASCII STL triangle readers.

use super::{Point, triangle_limit_message};
use crate::services::model_preview::{MAX_MODEL_TRIANGLES, ModelEncoding};

const BINARY_HEADER: usize = 84;
const BINARY_FACE: usize = 50;

pub(super) fn stl(bytes: &[u8]) -> Result<(Vec<[Point; 3]>, ModelEncoding), String> {
    if let Some(count) = binary_triangle_count(bytes)? {
        return binary_faces(&bytes[BINARY_HEADER..], count)
            .map(|faces| (faces, ModelEncoding::BinaryStl));
    }
    ascii_faces(bytes).map(|faces| (faces, ModelEncoding::AsciiStl))
}

/// The header's triangle count, when the file length matches it exactly.
fn binary_triangle_count(bytes: &[u8]) -> Result<Option<usize>, String> {
    if bytes.len() < BINARY_HEADER {
        return Ok(None);
    }
    let count = u32::from_le_bytes(
        bytes[80..BINARY_HEADER]
            .try_into()
            .map_err(|_| "Invalid STL")?,
    ) as usize;
    let exact = count
        .checked_mul(BINARY_FACE)
        .and_then(|n| n.checked_add(BINARY_HEADER))
        == Some(bytes.len());
    Ok(exact.then_some(count))
}

fn binary_faces(body: &[u8], count: usize) -> Result<Vec<[Point; 3]>, String> {
    if count > MAX_MODEL_TRIANGLES {
        return Err(triangle_limit_message());
    }
    let mut faces = Vec::with_capacity(count);
    for face in body.as_chunks::<BINARY_FACE>().0 {
        faces.push(binary_face(face)?);
    }
    Ok(faces)
}

/// Reads the three vertices that follow a face's normal.
fn binary_face(face: &[u8; BINARY_FACE]) -> Result<[Point; 3], String> {
    let mut points = [[0.; 3]; 3];
    for (index, coordinate) in points.as_flattened_mut().iter_mut().enumerate() {
        let start = 12 + index * 4;
        *coordinate = f32::from_le_bytes(
            face[start..start + 4]
                .try_into()
                .map_err(|_| "Invalid STL")?,
        );
        if !coordinate.is_finite() {
            return Err("Invalid STL coordinate".into());
        }
    }
    Ok(points)
}

fn ascii_coordinate(field: Option<&str>) -> Result<f32, String> {
    let value: f32 = field
        .ok_or("Invalid STL vertex")?
        .parse()
        .map_err(|_| "Invalid STL vertex")?;
    value
        .is_finite()
        .then_some(value)
        .ok_or("Invalid STL coordinate".into())
}

fn ascii_vertex(line: &str) -> Result<Option<Point>, String> {
    let mut fields = line.split_whitespace();
    if fields.next() != Some("vertex") {
        return Ok(None);
    }
    Ok(Some([
        ascii_coordinate(fields.next())?,
        ascii_coordinate(fields.next())?,
        ascii_coordinate(fields.next())?,
    ]))
}

fn ascii_faces(bytes: &[u8]) -> Result<Vec<[Point; 3]>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Invalid STL")?;
    if !text.trim_start().starts_with("solid") {
        return Err("Invalid STL".into());
    }
    let mut faces = Vec::new();
    let mut points = Vec::new();
    for line in text.lines() {
        let Some(point) = ascii_vertex(line)? else {
            continue;
        };
        points.push(point);
        if points.len() < 3 {
            continue;
        }
        if faces.len() >= MAX_MODEL_TRIANGLES {
            return Err(triangle_limit_message());
        }
        faces.push([points[0], points[1], points[2]]);
        points.clear();
    }
    if !points.is_empty() || faces.is_empty() {
        return Err("STL contains no valid triangles".into());
    }
    Ok(faces)
}
