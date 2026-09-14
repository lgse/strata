// SPDX-License-Identifier: MIT

use resvg::{tiny_skia, usvg};

/// Only bundled interface SVGs belong here, never files from a browser location.
pub(super) fn surface(source: &str, pixels: i32) -> Option<cairo::ImageSurface> {
    if !(1..=768).contains(&pixels) || source.len() > 64 * 1024 {
        return None;
    }
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(source, &options).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(pixels as u32, pixels as u32)?;
    let transform = tiny_skia::Transform::from_scale(
        pixels as f32 / tree.size().width(),
        pixels as f32 / tree.size().height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut data = pixmap.take();
    // tiny-skia uses premultiplied RGBA; Cairo uses native-endian premultiplied ARGB.
    for pixel in data.as_chunks_mut::<4>().0 {
        let argb = u32::from_be_bytes([pixel[3], pixel[0], pixel[1], pixel[2]]);
        pixel.copy_from_slice(&argb.to_ne_bytes());
    }
    cairo::ImageSurface::create_for_data(data, cairo::Format::ARgb32, pixels, pixels, pixels * 4)
        .ok()
}

#[cfg(test)]
mod tests;
