// SPDX-License-Identifier: MIT

/// Scales every pixel radius in `border-radius` declarations by `scale`.
/// Percentages stay as they are: they draw circles, not rounded corners.
pub(super) fn scale_radii(css: &str, scale: f64) -> String {
    let mut output = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("-radius:") {
        let after = start + "-radius:".len();
        let end = rest[after..]
            .find([';', '}'])
            .map_or(rest.len(), |offset| after + offset);
        output.push_str(&rest[..after]);
        output.push_str(&scale_lengths(&rest[after..end], scale));
        rest = &rest[end..];
    }
    output.push_str(rest);
    output
}

fn scale_lengths(value: &str, scale: f64) -> String {
    let mut scaled = String::with_capacity(value.len());
    for (index, token) in value.split(' ').enumerate() {
        if index > 0 {
            scaled.push(' ');
        }
        match token
            .strip_suffix("px")
            .and_then(|pixels| pixels.parse::<f64>().ok())
        {
            Some(pixels) => scaled.push_str(&format_pixels(pixels * scale)),
            None => scaled.push_str(token),
        }
    }
    scaled
}

fn format_pixels(pixels: f64) -> String {
    let rounded = (pixels * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 {
        format!("{}px", rounded as i64)
    } else {
        format!("{rounded}px")
    }
}

#[cfg(test)]
mod tests;
