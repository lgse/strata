// SPDX-License-Identifier: MIT

use std::{fs::File, io::BufReader, path::Path, process::Command, time::Duration};

use exif::{Exif, In, Tag, Value};

use crate::sandbox::{metadata::MAX_METADATA_BYTES, raw_metadata::RawMetadata};

use super::bounded_output_with_timeout;

const NEW_SUBFILE_TYPE: Tag = Tag(exif::Context::Tiff, 254);
const DEFAULT_CROP_SIZE: Tag = Tag(exif::Context::Tiff, 50720);

pub(crate) fn read(input: &Path) -> Result<Vec<u8>, String> {
    let mut metadata = RawMetadata::default();
    for tool in ["raw-identify", "dcraw"] {
        let mut command = Command::new(tool);
        command.env("LC_ALL", "C");
        if tool == "dcraw" {
            command.arg("-i");
        }
        command.arg("-v").arg(input);
        if let Ok(Some(output)) =
            bounded_output_with_timeout(&mut command, MAX_METADATA_BYTES, Duration::from_secs(3))
            && output.status.success()
            && let Ok(output) = String::from_utf8(output.stdout)
        {
            metadata = parse_identify(&output);
            if metadata != RawMetadata::default() {
                break;
            }
        }
    }
    // The identification tools omit GPS and some lens tags. Read EXIF inside
    // the same sandbox without decoding pixels or exposing arbitrary tags.
    if let Ok(file) = File::open(input)
        && let Ok(exif) = exif::Reader::new()
            .continue_on_error(true)
            .read_from_container(&mut BufReader::new(file))
            .or_else(|error| error.distill_partial_result(|_| {}))
    {
        let exif = parse_exif(&exif);
        metadata.dimensions = metadata.dimensions.or(exif.dimensions);
        metadata.camera = metadata.camera.or(exif.camera);
        metadata.lens = metadata.lens.or(exif.lens);
        metadata.focal_length = metadata.focal_length.or(exif.focal_length);
        metadata.shutter_speed = metadata.shutter_speed.or(exif.shutter_speed);
        metadata.iso = metadata.iso.or(exif.iso);
        metadata.gps = exif.gps;
    }
    let bytes = serde_json::to_vec(&metadata).map_err(|error| error.to_string())?;
    let metadata = RawMetadata::from_json(&bytes)?;
    serde_json::to_vec(&metadata).map_err(|error| error.to_string())
}

fn text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn number(value: &str) -> Option<f64> {
    let value = value.trim();
    let value = if let Some((numerator, denominator)) = value.split_once('/') {
        numerator.trim().parse::<f64>().ok()? / denominator.trim().parse::<f64>().ok()?
    } else {
        value.parse().ok()?
    };
    (value.is_finite() && value > 0.0).then_some(value)
}

fn dimensions(width: &str, height: &str) -> Option<(u32, u32)> {
    let width = width.trim().parse().ok()?;
    let height = height.trim().parse().ok()?;
    ((1..=1_000_000).contains(&width) && (1..=1_000_000).contains(&height))
        .then_some((width, height))
}

fn parse_identify(output: &str) -> RawMetadata {
    let mut metadata = RawMetadata::default();
    let mut full = None;
    let mut image = None;
    let mut rendered = None;
    let mut flip = None;
    for line in output.lines() {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            "Camera" => {
                let camera = value.split(" ID: ").next().unwrap_or(value);
                metadata.camera = text(match camera.split_once(' ') {
                    Some((make, model))
                        if model
                            .to_lowercase()
                            .strip_prefix(&make.to_lowercase())
                            .is_some_and(|rest| rest.starts_with(' ')) =>
                    {
                        model
                    }
                    _ => camera,
                });
            }
            "Lens" if metadata.lens.is_none() => metadata.lens = text(value),
            "ISO speed" => metadata.iso = number(value),
            "Shutter" => metadata.shutter_speed = number(value.trim_end_matches("sec").trim()),
            "Focal length" => metadata.focal_length = number(value.trim_end_matches("mm").trim()),
            "Full size" | "Image size" | "Output size" => {
                let size = value
                    .split_once('x')
                    .and_then(|(width, height)| dimensions(width, height));
                match key {
                    "Full size" => full = size,
                    "Image size" => image = size,
                    _ => rendered = size,
                }
            }
            "Image flip" => flip = value.parse::<u32>().ok(),
            _ => {}
        }
    }
    // Both tools report an already-oriented Output size.
    metadata.dimensions = rendered.or_else(|| {
        let size = image.or(full);
        if flip.is_some_and(|flip| flip & 4 != 0) {
            size.map(|(width, height)| (height, width))
        } else {
            size
        }
    });
    metadata
}

fn field(exif: &Exif, tag: Tag) -> Option<&Value> {
    Some(&exif.get_field(tag, In::PRIMARY)?.value)
}

fn exif_text(exif: &Exif, tag: Tag) -> Option<String> {
    let Value::Ascii(values) = field(exif, tag)? else {
        return None;
    };
    text(std::str::from_utf8(values.first()?).ok()?)
}

fn exif_number(value: &Value, index: usize) -> Option<f64> {
    let number = match value {
        Value::Rational(values) => values.get(index)?.to_f64(),
        Value::SRational(values) => values.get(index)?.to_f64(),
        _ => f64::from(value.get_uint(index)?),
    };
    number.is_finite().then_some(number)
}

fn coordinate(value: &Value, reference: &str, latitude: bool) -> Option<f64> {
    let Value::Rational(parts) = value else {
        return None;
    };
    let [degrees, minutes, seconds] = parts.as_slice() else {
        return None;
    };
    let (degrees, minutes, seconds) = (degrees.to_f64(), minutes.to_f64(), seconds.to_f64());
    if !degrees.is_finite() || !(0.0..60.0).contains(&minutes) || !(0.0..60.0).contains(&seconds) {
        return None;
    }
    let sign = match (latitude, reference) {
        (true, "N") | (false, "E") => 1.0,
        (true, "S") | (false, "W") => -1.0,
        _ => return None,
    };
    let value = degrees + minutes / 60.0 + seconds / 3600.0;
    (value <= if latitude { 90.0 } else { 180.0 }).then_some(sign * value)
}

fn parse_exif(exif: &Exif) -> RawMetadata {
    let numeric = |tag| field(exif, tag).and_then(|value| exif_number(value, 0));
    let dimensions = field(exif, DEFAULT_CROP_SIZE)
        .and_then(|crop| exif_number(crop, 0).zip(exif_number(crop, 1)))
        .or_else(|| numeric(Tag::PixelXDimension).zip(numeric(Tag::PixelYDimension)))
        .or_else(|| {
            // Reduced-resolution TIFF directories describe embedded thumbnails.
            (numeric(NEW_SUBFILE_TYPE).unwrap_or(0.0) as u32 & 1 == 0)
                .then(|| numeric(Tag::ImageWidth).zip(numeric(Tag::ImageLength)))
                .flatten()
        })
        .filter(|(width, height)| {
            (1.0..=1_000_000.0).contains(width) && (1.0..=1_000_000.0).contains(height)
        })
        .map(|(width, height)| (width.round() as u32, height.round() as u32));
    let dimensions = if matches!(numeric(Tag::Orientation), Some(5.0..=8.0)) {
        dimensions.map(|(width, height)| (height, width))
    } else {
        dimensions
    };
    let make = exif_text(exif, Tag::Make).unwrap_or_default();
    let model = exif_text(exif, Tag::Model).unwrap_or_default();
    let camera = if model.to_lowercase().starts_with(&make.to_lowercase()) {
        text(&model)
    } else {
        text(&format!("{make} {model}"))
    };
    let gps =
        field(exif, Tag::GPSLatitude)
            .and_then(|value| coordinate(value, &exif_text(exif, Tag::GPSLatitudeRef)?, true))
            .zip(field(exif, Tag::GPSLongitude).and_then(|value| {
                coordinate(value, &exif_text(exif, Tag::GPSLongitudeRef)?, false)
            }));
    RawMetadata {
        dimensions,
        camera,
        lens: exif_text(exif, Tag::LensModel),
        focal_length: numeric(Tag::FocalLength),
        shutter_speed: numeric(Tag::ExposureTime),
        iso: numeric(Tag::ISOSpeed)
            .or_else(|| numeric(Tag::PhotographicSensitivity).filter(|iso| *iso != 65535.0)),
        gps,
    }
}

#[cfg(test)]
mod tests;
