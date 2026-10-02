// SPDX-License-Identifier: MIT

use crate::services::image_conversion::{self, ImageKind, MAX_EDGE, MAX_INPUT_BYTES, MAX_PIXELS};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageEncoder, ImageReader, Limits};
use std::{
    fs,
    io::{BufReader, Cursor},
    path::Path,
};

#[cfg(test)]
mod tests;

pub(super) fn run(input: &Path, output: &Path, convert: bool) -> Result<(), String> {
    let result = process(input, convert);
    match result {
        Ok(bytes) => fs::write(output, bytes).map_err(|error| error.to_string()),
        Err(error) => {
            let _ = fs::write(output.with_file_name("result.error"), &error);
            Err(error)
        }
    }
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    // image treats this allocation budget as best-effort; pre-decode dimensions
    // and the sandbox's process limits remain the resource boundary.
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits
}

fn check_dimensions(decoder: &impl ImageDecoder) -> Result<(), String> {
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("The image exceeds the 16 megapixel conversion limit".into());
    }
    Ok(())
}

fn process(input: &Path, convert: bool) -> Result<Vec<u8>, String> {
    if fs::metadata(input)
        .map_err(|error| error.to_string())?
        .len()
        > MAX_INPUT_BYTES
    {
        return Err("The image exceeds the 32 MiB conversion limit".into());
    }
    let kind = image_conversion::detect(input)
        .ok_or("Only JPEG, BMP, static WebP and single-frame GIF can be converted to PNG")?;
    let file = || {
        fs::File::open(input)
            .map(BufReader::new)
            .map_err(|error| error.to_string())
    };
    let invalid = |error| match error {
        image::ImageError::Limits(_) => {
            "The image exceeds the conversion size or memory limits".to_owned()
        }
        _ => "The image is damaged or uses unsupported image data".to_owned(),
    };
    let animated = "Animated images cannot be converted to PNG. Choose a static image instead.";
    let mut profile = None;
    let image = if kind == ImageKind::Gif {
        let mut decoder = image::codecs::gif::GifDecoder::new(file()?).map_err(invalid)?;
        decoder
            .set_limits(limits())
            .map_err(|error| error.to_string())?;
        check_dimensions(&decoder)?;
        let mut frames = decoder.into_frames();
        let frame = frames
            .next()
            .ok_or("The GIF has no image")?
            .map_err(|error| error.to_string())?;
        match frames.next() {
            Some(Ok(_)) => return Err(animated.into()),
            Some(Err(error)) => return Err(format!("Could not read GIF frames: {error}")),
            None => {}
        }
        DynamicImage::ImageRgba8(frame.into_buffer())
    } else {
        if kind == ImageKind::WebP
            && image::codecs::webp::WebPDecoder::new(file()?)
                .map_err(invalid)?
                .has_animation()
        {
            return Err(animated.into());
        }
        // The chooser bypasses this helper for accepted PNGs. Keep the APNG check
        // and original-byte return as defense if ConvertImage is ever handed a PNG.
        if kind == ImageKind::Png
            && image::codecs::png::PngDecoder::with_limits(file()?, limits())
                .map_err(invalid)?
                .is_apng()
                .map_err(invalid)?
        {
            return Err(animated.into());
        }
        let mut reader = ImageReader::with_format(file()?, kind.format());
        reader.limits(limits());
        let mut decoder = reader.into_decoder().map_err(invalid)?;
        check_dimensions(&decoder)?;
        let orientation = decoder.orientation().map_err(invalid)?;
        profile = decoder.icc_profile().map_err(invalid)?;
        let mut image = DynamicImage::from_decoder(decoder).map_err(invalid)?;
        image.apply_orientation(orientation);
        image
    };
    if let Some(profile) = &profile
        && profile.get(36..40) == Some(b"acsp")
    {
        let colour_space: &[u8] =
            if matches!(image.color(), image::ColorType::L8 | image::ColorType::La8) {
                b"GRAY"
            } else {
                b"RGB "
            };
        if profile.get(16..20) != Some(colour_space) {
            return Err("This image's colour profile cannot be preserved in PNG. Choose an RGB or grayscale image.".into());
        }
    }
    if !convert {
        return serde_json::to_vec(&kind).map_err(|error| error.to_string());
    }
    if kind == ImageKind::Png {
        return fs::read(input).map_err(|error| error.to_string());
    }
    let mut bytes = Cursor::new(Vec::new());
    let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    if let Some(profile) = profile {
        encoder
            .set_icc_profile(profile)
            .map_err(|error| error.to_string())?;
    }
    encoder
        .write_image(
            image.as_bytes(),
            image.width(),
            image.height(),
            image.color().into(),
        )
        .map_err(invalid)?;
    if bytes.get_ref().len() as u64 > crate::sandbox::MAX_OUTPUT_BYTES {
        return Err("The converted PNG exceeds the 32 MiB output limit".into());
    }
    Ok(bytes.into_inner())
}
