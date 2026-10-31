// SPDX-License-Identifier: MIT

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::REPOSITORY;

/// A pixel differs when one channel moves by more than this.
const CHANNEL_TOLERANCE: u8 = 24;
pub(crate) const DIFFERENT_PIXEL_BUDGET: f64 = 0.005;
/// Set to `1` to write each capture as its committed baseline instead of comparing.
const UPDATE_BASELINES: &str = "STRATA_QA_UPDATE_BASELINES";

#[derive(Debug, PartialEq)]
pub(crate) struct Comparison {
    name: String,
    pub(crate) matched: bool,
    pub(crate) different_fraction: f64,
    pub(crate) detail: String,
}

impl Comparison {
    pub(crate) fn failure(&self, artifacts: &Path) -> String {
        format!(
            "{}: {} ({:.3}% of pixels differ, budget {:.3}%)\n\
             Expected, actual, and diff images are in {}\n\
             Regenerate deliberately with STRATA_QA_UPDATE_BASELINES=1 ./scripts/qa.sh <scenario>.",
            self.name,
            self.detail,
            self.different_fraction * 100.0,
            DIFFERENT_PIXEL_BUDGET * 100.0,
            artifacts.display()
        )
    }
}

fn baseline_directory() -> PathBuf {
    Path::new(REPOSITORY)
        .join("tests/qa/baselines")
        .join(format!(
            "gtk-{}.{}",
            gtk::major_version(),
            gtk::minor_version()
        ))
}

/// Writes a baseline only when `STRATA_QA_UPDATE_BASELINES=1`.
pub(crate) fn compare_to_baseline(
    name: &str,
    actual: &Path,
    artifacts: &Path,
) -> Result<Comparison, String> {
    let update = std::env::var_os(UPDATE_BASELINES).is_some_and(|value| value == "1");
    compare_in(&baseline_directory(), name, actual, artifacts, update)
}

pub(crate) fn compare_in(
    directory: &Path,
    name: &str,
    actual: &Path,
    artifacts: &Path,
    update: bool,
) -> Result<Comparison, String> {
    let expected = directory.join(format!("{name}.png"));
    fs::copy(actual, artifacts.join(format!("{name}.actual.png")))
        .map_err(|error| format!("cannot keep the capture: {error}"))?;
    let comparison = |matched, different_fraction, detail| Comparison {
        name: name.to_owned(),
        matched,
        different_fraction,
        detail,
    };
    if update {
        fs::create_dir_all(directory)
            .and_then(|()| fs::copy(actual, &expected))
            .map_err(|error| format!("cannot write the baseline: {error}"))?;
        return Ok(comparison(true, 0.0, "updated".to_owned()));
    }
    if !expected.exists() {
        let profile = directory.file_name().unwrap_or_default().to_string_lossy();
        return Ok(comparison(
            false,
            1.0,
            format!("no baseline at {profile}/{name}.png"),
        ));
    }
    let open = |path: &Path| {
        image::open(path)
            .map(|image| image.to_rgb8())
            .map_err(|error| format!("cannot read {}: {error}", path.display()))
    };
    let (baseline, candidate) = (open(&expected)?, open(actual)?);
    if baseline.dimensions() != candidate.dimensions() {
        return Ok(comparison(
            false,
            1.0,
            format!(
                "size changed from {:?} to {:?}",
                baseline.dimensions(),
                candidate.dimensions()
            ),
        ));
    }
    let mask = differing_pixels(&baseline, &candidate);
    let fraction = different_fraction(&mask);
    let matched = fraction <= DIFFERENT_PIXEL_BUDGET;
    if !matched {
        diff_image(&baseline, &candidate, &mask)
            .save_with_format(
                artifacts.join(format!("{name}.diff.png")),
                image::ImageFormat::Png,
            )
            .map_err(|error| format!("cannot write the diff: {error}"))?;
        fs::copy(&expected, artifacts.join(format!("{name}.expected.png")))
            .map_err(|error| format!("cannot keep the baseline: {error}"))?;
    }
    let detail = if matched {
        "matched"
    } else {
        "differs from the committed baseline"
    };
    Ok(comparison(matched, fraction, detail.to_owned()))
}

pub(crate) fn differing_pixels(
    baseline: &image::RgbImage,
    candidate: &image::RgbImage,
) -> Vec<bool> {
    baseline
        .pixels()
        .zip(candidate.pixels())
        .map(|(before, after)| {
            before
                .0
                .iter()
                .zip(after.0)
                .any(|(left, right)| left.abs_diff(right) > CHANNEL_TOLERANCE)
        })
        .collect()
}

pub(crate) fn different_fraction(mask: &[bool]) -> f64 {
    let different = mask.iter().filter(|differs| **differs).count();
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel counts stay far below 2^52"
    )]
    let fraction = different as f64 / mask.len() as f64;
    fraction
}

fn diff_image(
    baseline: &image::RgbImage,
    candidate: &image::RgbImage,
    mask: &[bool],
) -> image::RgbImage {
    let mut blended = baseline.clone();
    for ((pixel, after), differs) in blended.pixels_mut().zip(candidate.pixels()).zip(mask) {
        pixel.0 = if *differs {
            [255, 0, 128]
        } else {
            std::array::from_fn(|channel| {
                let (left, right) = (pixel.0[channel], after.0[channel]);
                left / 2 + right / 2 + (left & right & 1)
            })
        };
    }
    blended
}
