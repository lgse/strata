// SPDX-License-Identifier: MIT

use std::{fs::File, io::Read, path::Path};

use crate::services::{
    ModelPalette,
    route_preview::{
        INVALID_ROUTE, MAX_ROUTE_INPUT_BYTES, MAX_ROUTE_POINTS, RouteFormat, RoutePreview,
    },
};

mod fit;
mod geometry;
mod render;
mod xml;

#[derive(Clone, Copy, Debug)]
struct Point {
    lat: f64,
    lon: f64,
    elevation: Option<f64>,
}

#[derive(Default, Debug)]
struct Route {
    segments: Vec<Vec<Point>>,
    points: usize,
    new_segment: bool,
    timestamps: Option<[i64; 2]>,
}

impl Route {
    fn record_timestamp(&mut self, timestamp: i64) {
        let [first, last] = self.timestamps.get_or_insert([timestamp, timestamp]);
        *first = (*first).min(timestamp);
        *last = (*last).max(timestamp);
    }

    fn duration_seconds(&self) -> Option<u64> {
        let [first, last] = self.timestamps?;
        u64::try_from(last.checked_sub(first)?)
            .ok()
            .filter(|seconds| *seconds > 0)
    }

    fn split(&mut self) {
        self.new_segment = true;
    }

    fn push(&mut self, point: Option<Point>) -> Result<(), String> {
        let Some(point) = point.filter(|p| {
            p.lat.is_finite()
                && p.lon.is_finite()
                && (-90.0..=90.0).contains(&p.lat)
                && (-180.0..=180.0).contains(&p.lon)
        }) else {
            self.split();
            return Ok(());
        };
        if self.points >= MAX_ROUTE_POINTS {
            return Err("This route exceeds the 250,000 point preview limit.".into());
        }
        if self.new_segment || self.segments.is_empty() {
            self.segments.push(Vec::new());
            self.new_segment = false;
        }
        if let Some(segment) = self.segments.last_mut() {
            segment.push(point);
        }
        self.points += 1;
        Ok(())
    }
}

fn elevation(value: f64) -> Option<f64> {
    (value.is_finite() && (-20_000.0..=100_000.0).contains(&value)).then_some(value)
}

fn parse(bytes: &[u8], format: RouteFormat) -> Result<Route, String> {
    if bytes.len() as u64 > MAX_ROUTE_INPUT_BYTES {
        return Err("This route file exceeds the 32 MiB preview limit.".into());
    }
    let route = match format {
        RouteFormat::Gpx | RouteFormat::Kml => xml::parse(bytes, format),
        RouteFormat::Fit => fit::parse(bytes),
    }?;
    if route.points == 0 {
        return Err(INVALID_ROUTE.into());
    }
    Ok(route)
}

pub(super) fn render(input: &Path, argument: &str) -> Result<Vec<u8>, String> {
    let mut parts = argument.split(':');
    let format = match parts.next() {
        Some("GPX") => RouteFormat::Gpx,
        Some("FIT") => RouteFormat::Fit,
        Some("KML") => RouteFormat::Kml,
        _ => return Err(INVALID_ROUTE.into()),
    };
    let color = |part: Option<&str>| {
        part.and_then(|s| u32::from_str_radix(s, 16).ok())
            .filter(|n| *n <= 0xffffff)
            .ok_or_else(|| INVALID_ROUTE.to_owned())
    };
    let palette = ModelPalette {
        accent: color(parts.next())?,
        surface: color(parts.next())?,
    };
    if parts.next().is_some() {
        return Err(INVALID_ROUTE.into());
    }
    let mut bytes = Vec::new();
    File::open(input)
        .map_err(|e| e.to_string())?
        .take(MAX_ROUTE_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let route = parse(&bytes, format)?;
    let preview: RoutePreview = render::draw(&route, palette)?;
    serde_json::to_vec(&preview).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
