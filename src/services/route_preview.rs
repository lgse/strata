// SPDX-License-Identifier: MIT

use std::{ffi::OsStr, path::Path};

use super::ModelPalette;

pub(crate) const MAX_ROUTE_INPUT_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_ROUTE_POINTS: usize = 250_000;
pub(crate) const ROUTE_WIDTH: u32 = 800;
pub(crate) const ROUTE_HEIGHT: u32 = 600;
pub(crate) const PROFILE_HEIGHT: u32 = 70;
pub(crate) const INVALID_ROUTE: &str =
    "This route file is invalid or contains no supported coordinates.";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RouteFormat {
    Gpx,
    Fit,
    Kml,
}

impl RouteFormat {
    pub(crate) fn for_name(name: &OsStr) -> Option<Self> {
        match Path::new(name)
            .extension()?
            .to_str()?
            .to_ascii_lowercase()
            .as_str()
        {
            "gpx" => Some(Self::Gpx),
            "fit" => Some(Self::Fit),
            "kml" => Some(Self::Kml),
            _ => None,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Gpx => "GPX",
            Self::Fit => "FIT",
            Self::Kml => "KML",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RouteRender {
    pub format: RouteFormat,
    pub palette: ModelPalette,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutePreview {
    pub png: Vec<u8>,
    pub profile: Option<Vec<u8>>,
    pub distance_m: f64,
    pub duration_seconds: Option<u64>,
    pub elevation: Option<[f64; 2]>,
    pub points: usize,
    pub samples: Vec<RouteSample>,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteSample {
    #[serde(rename = "xy")]
    pub position: [f32; 2],
    #[serde(rename = "d")]
    pub distance_m: f64,
    #[serde(rename = "e")]
    pub elevation_y: Option<f32>,
    #[serde(rename = "s")]
    pub starts_segment: bool,
}

impl RoutePreview {
    fn valid_samples(&self) -> bool {
        if self.samples.len() != self.points
            || self
                .samples
                .first()
                .is_none_or(|p| !p.starts_segment || p.distance_m != 0.0)
            || self
                .samples
                .last()
                .is_none_or(|p| p.distance_m != self.distance_m)
        {
            return false;
        }
        let mut previous_distance = 0.0;
        self.samples.iter().all(|sample| {
            let valid = sample
                .position
                .iter()
                .zip([ROUTE_WIDTH as f32, ROUTE_HEIGHT as f32])
                .all(|(value, maximum)| value.is_finite() && (0.0..=maximum).contains(value))
                && sample.distance_m.is_finite()
                && (previous_distance..=self.distance_m).contains(&sample.distance_m)
                && (!sample.starts_segment || sample.distance_m == previous_distance)
                && sample.elevation_y.is_none_or(|y| {
                    self.profile.is_some()
                        && y.is_finite()
                        && (0.0..=PROFILE_HEIGHT as f32).contains(&y)
                });
            previous_distance = sample.distance_m;
            valid
        })
    }

    pub(crate) fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let invalid = || "The preview renderer produced invalid image data".to_owned();
        let result: Self = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        let valid_png =
            |png: &[u8], height| crate::sandbox::png_dimensions(png) == Some((ROUTE_WIDTH, height));
        if !valid_png(&result.png, ROUTE_HEIGHT)
            || result
                .profile
                .as_ref()
                .is_some_and(|png| !valid_png(png, PROFILE_HEIGHT))
            || result.profile.is_some() != result.elevation.is_some()
            || result
                .duration_seconds
                .is_some_and(|seconds| seconds == 0 || seconds > i64::MAX as u64 / 1_000_000)
            || !result.distance_m.is_finite()
            || !(0.0..=MAX_ROUTE_POINTS as f64 * 21_000_000.0).contains(&result.distance_m)
            || !(1..=MAX_ROUTE_POINTS).contains(&result.points)
            || !result.valid_samples()
            || result.elevation.is_some_and(|[low, high]| {
                !low.is_finite()
                    || !high.is_finite()
                    || low > high
                    || low < -20_000.0
                    || high > 100_000.0
            })
        {
            return Err(invalid());
        }
        Ok(result)
    }
}

pub(crate) fn profile_x(distance_m: f64, total_m: f64) -> f64 {
    2.0 + if total_m > 0.0 {
        distance_m / total_m * (f64::from(ROUTE_WIDTH) - 4.0)
    } else {
        0.0
    }
}

pub(crate) fn profile_distance(x: f64, total_m: f64) -> f64 {
    ((x - 2.0) / (f64::from(ROUTE_WIDTH) - 4.0)).clamp(0.0, 1.0) * total_m
}

pub(crate) fn profile_elevation(y: f32, [low, high]: [f64; 2]) -> f64 {
    let height = f64::from(PROFILE_HEIGHT);
    low + ((height - 4.0 - f64::from(y)) / (height - 8.0)).clamp(0.0, 1.0) * (high - low)
}
