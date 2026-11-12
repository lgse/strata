// SPDX-License-Identifier: MIT
//! Software rasterizer for model previews.

use resvg::tiny_skia::{Color, Pixmap};

use super::{Point, TOO_COMPLEX_MODEL_MESSAGE};
use crate::services::{
    ModelPreviewStage,
    model_preview::{MAX_MODEL_RASTER_WORK, ModelView},
};

/// The canvas, colors and camera a model preview is drawn with.
#[derive(Clone, Copy)]
pub(super) struct Shading {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) accent: [u8; 3],
    pub(super) surface: [u8; 4],
    pub(super) view: ModelView,
}

fn bounds(faces: &[[Point; 3]]) -> Result<(Point, Point), String> {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in faces.iter().flatten() {
        if p.iter().any(|value| !value.is_finite()) {
            return Err("Invalid transformed model coordinate".into());
        }
        for axis in 0..3 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    Ok((min, max))
}

/// Projects model space onto a square canvas, fitting every camera to one box.
struct Camera {
    center: Point,
    scale: f32,
    sx: f32,
    sy: f32,
    side: u32,
}

impl Camera {
    fn fit(faces: &[[Point; 3]], side: u32, view: ModelView) -> Result<Self, String> {
        let (min, max) = bounds(faces)?;
        let span: Point = std::array::from_fn(|axis| max[axis] - min[axis]);
        let center: Point = std::array::from_fn(|axis| min[axis] + span[axis] * 0.5);
        // Fit the same bounding box for all four cameras so switching never zooms.
        let extent = ((span[0] + span[1]) * std::f32::consts::FRAC_1_SQRT_2)
            .max((span[0] + span[1] + 2. * span[2]) / 6_f32.sqrt());
        if !extent.is_finite() || extent <= 0. {
            return Err("Model has no visible extent".into());
        }
        let [sx, sy, _] = view.camera_direction();
        Ok(Self {
            center,
            scale: (side as f32 * 0.82) / extent,
            sx,
            sy,
            side,
        })
    }

    fn rotate(&self, face: [Point; 3]) -> [Point; 3] {
        let (sx, sy) = (self.sx, self.sy);
        face.map(|p| {
            let [x, y, z] = std::array::from_fn(|axis| p[axis] - self.center[axis]);
            [
                (-sy * x + sx * y) * std::f32::consts::FRAC_1_SQRT_2,
                (-sx * x - sy * y + 2. * z) / 6_f32.sqrt(),
                (sx * x + sy * y + z) / 3_f32.sqrt(),
            ]
        })
    }

    fn to_screen(&self, points: [Point; 3]) -> [Point; 3] {
        points.map(|point| {
            [
                self.side as f32 * 0.5 + point[0] * self.scale,
                self.side as f32 * 0.5 - point[1] * self.scale,
                point[2],
            ]
        })
    }
}

fn difference(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn light(points: &[Point; 3]) -> f32 {
    let a = difference(points[1], points[0]);
    let b = difference(points[2], points[0]);
    let normal = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let length = (normal.iter().map(|n| n * n).sum::<f32>()).sqrt();
    if length > 0. {
        (0.58 + 0.36 * (normal[0] * 0.3 - normal[1] * 0.5 + normal[2] * 0.8).abs() / length)
            .clamp(0.35, 0.94)
    } else {
        0.58
    }
}

fn face_color(accent: [u8; 3], surface: [u8; 4], light: f32) -> [u8; 4] {
    let channel = |index: usize| {
        (accent[index] as f32 * light + surface[index] as f32 * (1. - light) * 0.35) as u8
    };
    [channel(0), channel(1), channel(2), 255]
}

fn edge(a: Point, b: Point, p: Point) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

fn lower(face: &[Point; 3], axis: usize) -> u32 {
    face.iter()
        .map(|p| p[axis])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.) as u32
}

fn upper(face: &[Point; 3], axis: usize, bound: u32) -> u32 {
    face.iter()
        .map(|p| p[axis])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .max(0.)
        .min(bound as f32) as u32
}

/// A screen-space triangle ready to be filled.
struct Triangle {
    face: [Point; 3],
    area: f32,
    color: [u8; 4],
}

impl Triangle {
    /// The depth at a pixel centre, when the triangle covers it.
    fn depth_at(&self, x: u32, y: u32) -> Option<f32> {
        let point = [x as f32 + 0.5, y as f32 + 0.5, 0.];
        let face = self.face;
        let weights = [
            edge(face[1], face[2], point) / self.area,
            edge(face[2], face[0], point) / self.area,
            edge(face[0], face[1], point) / self.area,
        ];
        if weights.iter().any(|weight| *weight < 0.) {
            return None;
        }
        Some(
            weights
                .iter()
                .zip(face)
                .map(|(weight, point)| weight * point[2])
                .sum(),
        )
    }
}

struct Raster {
    pixmap: Pixmap,
    depths: Vec<f32>,
    side: u32,
    work: u64,
}

impl Raster {
    fn new(side: u32, surface: [u8; 4]) -> Result<Self, String> {
        let mut pixmap = Pixmap::new(side, side).ok_or("Cannot allocate model preview")?;
        pixmap.fill(Color::from_rgba8(
            surface[0], surface[1], surface[2], surface[3],
        ));
        Ok(Self {
            pixmap,
            depths: vec![f32::NEG_INFINITY; (side * side) as usize],
            side,
            work: 0,
        })
    }

    fn fill(&mut self, triangle: &Triangle) -> Result<(), String> {
        let face = &triangle.face;
        let (left, right) = (lower(face, 0), upper(face, 0, self.side));
        let (top, bottom) = (lower(face, 1), upper(face, 1, self.side));
        self.work += u64::from(right.saturating_sub(left)) * u64::from(bottom.saturating_sub(top));
        if self.work > MAX_MODEL_RASTER_WORK {
            return Err(TOO_COMPLEX_MODEL_MESSAGE.into());
        }
        for y in top..bottom {
            for x in left..right {
                self.plot(triangle, x, y);
            }
        }
        Ok(())
    }

    fn plot(&mut self, triangle: &Triangle, x: u32, y: u32) {
        let Some(depth) = triangle.depth_at(x, y) else {
            return;
        };
        let index = (y * self.side + x) as usize;
        if depth > self.depths[index] {
            self.depths[index] = depth;
            self.pixmap.data_mut()[index * 4..index * 4 + 4].copy_from_slice(&triangle.color);
        }
    }
}

pub(super) fn shade(
    faces: &[[Point; 3]],
    shading: &Shading,
    progress: &dyn Fn(ModelPreviewStage),
) -> Result<Vec<u8>, String> {
    // Keep empty canvas from separating the model and its centred player controls.
    let side = shading.width.min(shading.height);
    progress(ModelPreviewStage::Rendering {
        triangles: faces.len(),
    });
    let camera = Camera::fit(faces, side, shading.view)?;
    let mut raster = Raster::new(side, shading.surface)?;
    for face in faces {
        let points = camera.rotate(*face);
        let light = light(&points);
        let face = camera.to_screen(points);
        let area = edge(face[0], face[1], face[2]);
        if area.abs() < 1e-8 {
            continue;
        }
        raster.fill(&Triangle {
            face,
            area,
            color: face_color(shading.accent, shading.surface, light),
        })?;
    }
    progress(ModelPreviewStage::Finishing);
    raster
        .pixmap
        .encode_png()
        .map_err(|error| error.to_string())
}
