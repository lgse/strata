// SPDX-License-Identifier: MIT

use std::{path::Path, process::Command, time::Duration};

use crate::sandbox::MAX_OUTPUT_BYTES;

use super::bounded_output_with_timeout;

const AUDIO_TIMEOUT: Duration = Duration::from_secs(4);
const AUDIO_SAMPLE_SECONDS: &str = "5";
const SPECTRUM_WIDTH: usize = 128;
const SPECTRUM_HEIGHT: usize = 64;
const BAR_COUNT: usize = 32;

/// Produces a color-neutral mask from the opening seconds for the UI to theme.
pub(super) fn render(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    spectrum(path, size)
}

fn spectrum(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let filter = format!(
        "[0:a]showspectrumpic=s={SPECTRUM_WIDTH}x{SPECTRUM_HEIGHT}:\
         legend=disabled:scale=cbrt:fscale=log:color=intensity:gain=1,\
         format=gray[out]"
    );
    let output = bounded_output_with_timeout(
        Command::new("ffmpeg")
            .args(["-v", "error", "-t", AUDIO_SAMPLE_SECONDS, "-i"])
            .arg(path)
            .args([
                "-filter_complex",
                &filter,
                "-map",
                "[out]",
                "-frames:v",
                "1",
                "-pix_fmt",
                "gray",
                "-f",
                "rawvideo",
                "-",
            ]),
        MAX_OUTPUT_BYTES,
        AUDIO_TIMEOUT,
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "Audio spectrum timed out".to_owned())?;
    if !output.status.success() || output.stdout.len() != SPECTRUM_WIDTH * SPECTRUM_HEIGHT {
        return Err("No audio stream for a spectrum".to_owned());
    }
    render_visualizer(&output.stdout, size)
}

fn render_visualizer(spectrum: &[u8], size: i32) -> Result<Vec<u8>, String> {
    let size = size.max(16);
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, size, size)
        .map_err(|error| error.to_string())?;
    let context = cairo::Context::new(&surface).map_err(|error| error.to_string())?;
    context.set_operator(cairo::Operator::Clear);
    context.paint().map_err(|error| error.to_string())?;
    context.set_operator(cairo::Operator::Over);
    context.set_source_rgb(1.0, 1.0, 1.0);

    let peaks = (0..BAR_COUNT)
        .rev()
        .map(|index| {
            let first_row = index * SPECTRUM_HEIGHT / BAR_COUNT;
            let last_row = (index + 1) * SPECTRUM_HEIGHT / BAR_COUNT;
            let pixels = &spectrum[first_row * SPECTRUM_WIDTH..last_row * SPECTRUM_WIDTH];
            pixels.iter().map(|value| f64::from(*value)).sum::<f64>() / pixels.len() as f64
        })
        .collect::<Vec<_>>();
    let global_peak = peaks.iter().copied().fold(1.0_f64, f64::max);
    let edge = f64::from(size);
    let left = edge * 0.10;
    let visualizer_width = edge * 0.80;
    let step = visualizer_width / BAR_COUNT as f64;
    let bar_width = step * 0.58;
    let min_half_height = edge * 0.025;
    let max_half_height = edge * 0.29;
    let center = edge / 2.0;

    for (index, peak) in peaks.into_iter().enumerate() {
        let strength = (peak / global_peak).sqrt();
        let half_height = min_half_height + strength * (max_half_height - min_half_height);
        let x = left + index as f64 * step + (step - bar_width) / 2.0;
        rounded_rectangle(
            &context,
            x,
            center - half_height,
            bar_width,
            half_height * 2.0,
            bar_width / 2.0,
        );
        context.fill().map_err(|error| error.to_string())?;
    }

    let mut png = Vec::new();
    surface
        .write_to_png(&mut png)
        .map_err(|error| error.to_string())?;
    Ok(png)
}

fn rounded_rectangle(
    context: &cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
) {
    use std::f64::consts::{FRAC_PI_2, PI};

    context.new_sub_path();
    context.arc(x + width - radius, y + radius, radius, -FRAC_PI_2, 0.0);
    context.arc(
        x + width - radius,
        y + height - radius,
        radius,
        0.0,
        FRAC_PI_2,
    );
    context.arc(x + radius, y + height - radius, radius, FRAC_PI_2, PI);
    context.arc(x + radius, y + radius, radius, PI, PI + FRAC_PI_2);
    context.close_path();
}
