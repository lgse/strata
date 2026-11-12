// SPDX-License-Identifier: MIT
//! 3MF package relationships and model XML, flattened into world-space faces.

use std::{collections::HashMap, fs, io::Read};

use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};

use super::{MULTIPART_MODEL_MESSAGE, Matrix, Point, VERTEX_LIMIT_MESSAGE, triangle_limit_message};
use crate::services::model_preview::*;

const IDENTITY: Matrix = [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.];
const MODEL_RELATIONSHIP: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";
const THUMBNAIL_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail";

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

/// The objects and build items read from a 3MF model part, with their running limits.
#[derive(Default)]
struct ModelParts {
    objects: HashMap<u32, Object>,
    current: Option<u32>,
    items: Vec<(u32, Matrix)>,
    vertices: usize,
    triangles: usize,
    components: usize,
}

impl ModelParts {
    fn read(xml: &[u8]) -> Result<Self, String> {
        let mut reader = Reader::from_reader(xml);
        let mut parts = Self::default();
        loop {
            let event = reader.read_event().map_err(|_| "Invalid 3MF model XML")?;
            match event {
                Event::Start(tag) | Event::Empty(tag) => parts.tag(&tag)?,
                Event::End(tag) if tag.local_name().as_ref() == b"object" => parts.current = None,
                Event::Eof => break,
                _ => {}
            }
        }
        Ok(parts)
    }

    fn tag(&mut self, tag: &BytesStart<'_>) -> Result<(), String> {
        let in_object = self.current.is_some();
        match tag.local_name().as_ref() {
            b"object" => self.object(tag),
            b"vertex" if in_object => self.vertex(tag),
            b"triangle" if in_object => self.triangle(tag),
            b"component" if in_object => self.component(tag),
            b"item" => self.item(tag),
            _ => Ok(()),
        }
    }

    fn current_object(&mut self) -> Result<&mut Object, String> {
        Ok(self
            .objects
            .get_mut(&self.current.unwrap_or_default())
            .ok_or("Invalid 3MF object")?)
    }

    fn object(&mut self, tag: &BytesStart<'_>) -> Result<(), String> {
        let id = index(tag, b"id")?;
        if self.objects.len() >= MAX_3MF_OBJECTS
            || self.objects.insert(id, Object::default()).is_some()
        {
            return Err("3MF object limit exceeded".into());
        }
        self.current = Some(id);
        Ok(())
    }

    fn vertex(&mut self, tag: &BytesStart<'_>) -> Result<(), String> {
        self.vertices += 1;
        if self.vertices > MAX_MODEL_VERTICES {
            return Err(VERTEX_LIMIT_MESSAGE.to_owned());
        }
        let point = [number(tag, b"x")?, number(tag, b"y")?, number(tag, b"z")?];
        self.current_object()?.vertices.push(point);
        Ok(())
    }

    fn triangle(&mut self, tag: &BytesStart<'_>) -> Result<(), String> {
        self.triangles += 1;
        if self.triangles > MAX_MODEL_TRIANGLES {
            return Err(triangle_limit_message());
        }
        let indices = [
            index(tag, b"v1")? as usize,
            index(tag, b"v2")? as usize,
            index(tag, b"v3")? as usize,
        ];
        self.current_object()?.triangles.push(indices);
        Ok(())
    }

    fn component(&mut self, tag: &BytesStart<'_>) -> Result<(), String> {
        self.components += 1;
        if self.components > MAX_MODEL_COMPONENT_REFERENCES {
            return Err("3MF component reference limit exceeded".into());
        }
        if attribute(tag, b"path")?.is_some() {
            return Err(MULTIPART_MODEL_MESSAGE.into());
        }
        let id = index(tag, b"objectid")?;
        let matrix = matrix(attribute(tag, b"transform")?)?;
        self.current_object()?.components.push((id, matrix));
        Ok(())
    }

    fn item(&mut self, tag: &BytesStart<'_>) -> Result<(), String> {
        if attribute(tag, b"path")?.is_some() {
            return Err(MULTIPART_MODEL_MESSAGE.into());
        }
        if self.items.len() >= MAX_3MF_BUILD_ITEMS {
            return Err("3MF build item limit exceeded".into());
        }
        self.items.push((
            index(tag, b"objectid")?,
            matrix(attribute(tag, b"transform")?)?,
        ));
        Ok(())
    }

    /// Expands the build items through their components into transformed faces.
    fn faces(self) -> Result<Vec<[Point; 3]>, String> {
        let mut faces = Vec::new();
        let mut stack: Vec<_> = self.items.into_iter().map(|(id, m)| (id, m, 0)).collect();
        let mut expanded = 0;
        while let Some((id, matrix, depth)) = stack.pop() {
            expanded += 1;
            if expanded > MAX_MODEL_COMPONENT_EXPANSIONS {
                return Err("3MF component expansion limit exceeded".into());
            }
            if depth > MAX_3MF_COMPONENT_DEPTH {
                return Err("3MF component nesting limit exceeded".into());
            }
            let object = self
                .objects
                .get(&id)
                .ok_or("3MF references a missing object")?;
            if expanded + stack.len() + object.components.len() > MAX_MODEL_COMPONENT_EXPANSIONS {
                return Err("3MF component expansion limit exceeded".into());
            }
            for &(id, child) in &object.components {
                stack.push((id, combine(matrix, child), depth + 1));
            }
            push_object_faces(object, matrix, &mut faces)?;
        }
        Ok(faces)
    }
}

fn push_object_faces(
    object: &Object,
    matrix: Matrix,
    faces: &mut Vec<[Point; 3]>,
) -> Result<(), String> {
    for indices in &object.triangles {
        let [Some(a), Some(b), Some(c)] = indices.map(|index| object.vertices.get(index).copied())
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
    Ok(())
}

pub(super) fn triangles_3mf(xml: &[u8]) -> Result<Vec<[Point; 3]>, String> {
    let faces = ModelParts::read(xml)?.faces()?;
    if faces.is_empty() {
        return Err("3MF contains no printable triangles".into());
    }
    Ok(faces)
}

enum PartKind {
    Model,
    Thumbnail,
}

fn read_relationships_xml(
    archive: &mut zip::ZipArchive<fs::File>,
) -> Result<Option<Vec<u8>>, String> {
    let file = match archive.by_name("_rels/.rels") {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
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
    Ok(Some(xml))
}

fn is_unsafe_part_reference(target: &str) -> bool {
    if target.is_empty() || target.contains(['\\', ':', '?', '#']) {
        return true;
    }
    target
        .split('/')
        .any(|part| matches!(part, ".." | "." | ""))
}

/// The model or thumbnail part a relationship targets; other relationships are skipped.
fn relationship(tag: &BytesStart<'_>) -> Result<Option<(PartKind, String)>, String> {
    let kind = match attribute(tag, b"Type")?.unwrap_or_default().as_str() {
        MODEL_RELATIONSHIP => PartKind::Model,
        THUMBNAIL_RELATIONSHIP => PartKind::Thumbnail,
        _ => return Ok(None),
    };
    if attribute(tag, b"TargetMode")?.as_deref() == Some("External") {
        if matches!(kind, PartKind::Model) {
            return Err("External 3MF model references are not supported".into());
        }
        return Ok(None);
    }
    let target = attribute(tag, b"Target")?.ok_or("Missing 3MF relationship target")?;
    let target =
        quick_xml::escape::unescape(&target).map_err(|_| "Invalid 3MF relationship target")?;
    let target = target.trim_start_matches('/');
    if is_unsafe_part_reference(target) {
        return Err("Invalid 3MF package part reference".into());
    }
    Ok(Some((kind, target.to_owned())))
}

#[derive(Default)]
struct Relationships {
    model: Option<String>,
    thumbnails: Vec<String>,
}

impl Relationships {
    fn add(&mut self, kind: PartKind, target: String) -> Result<(), String> {
        match kind {
            PartKind::Model => match self.model.replace(target) {
                Some(_) => Err("3MF package contains multiple root model relationships".into()),
                None => Ok(()),
            },
            PartKind::Thumbnail => {
                self.thumbnails.push(target);
                Ok(())
            }
        }
    }
}

pub(super) fn package_relationships(
    archive: &mut zip::ZipArchive<fs::File>,
) -> Result<(Option<String>, Vec<String>), String> {
    let Some(xml) = read_relationships_xml(archive)? else {
        return Ok((None, Vec::new()));
    };
    let mut reader = Reader::from_reader(xml.as_slice());
    let mut relationships = Relationships::default();
    loop {
        let event = reader
            .read_event()
            .map_err(|_| "Invalid 3MF package relationships")?;
        let tag = match event {
            Event::Start(tag) | Event::Empty(tag) => tag,
            Event::Eof => break,
            _ => continue,
        };
        if tag.local_name().as_ref() != b"Relationship" {
            continue;
        }
        if let Some((kind, target)) = relationship(&tag)? {
            relationships.add(kind, target)?;
        }
    }
    if relationships.model.is_none() {
        return Err("3MF package has no root model relationship".into());
    }
    Ok((relationships.model, relationships.thumbnails))
}
