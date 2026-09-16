// SPDX-License-Identifier: MIT

use resvg::{tiny_skia, usvg};
use std::{fs, path::Path};

pub(super) fn image(path: &Path) -> Result<Vec<u8>, String> {
    if path.extension().is_some_and(|ext| ext == "svg") {
        let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
        svg(&source)
    } else {
        super::render_raw(path, 800)
    }
}

pub(super) fn mermaid(path: &Path) -> Result<Vec<u8>, String> {
    let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
    if source.len() > crate::services::document_media::DIAGRAM_INPUT_LIMIT
        || source.lines().count() > 256
    {
        return Err("Mermaid preview limit exceeded".into());
    }
    let source = mermaid_rs_renderer::render(&source).map_err(|error| error.to_string())?;
    if source.len() > 2 * 1024 * 1024 {
        return Err("Mermaid output limit exceeded".into());
    }
    svg(&source)
}

fn svg(source: &str) -> Result<Vec<u8>, String> {
    let mut options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(source, &options).map_err(|error| error.to_string())?;
    let size = tree.size();
    let scale = (800.0 / size.width().max(size.height())).min(1.0);
    let width = (size.width() * scale).ceil().clamp(1.0, 800.0) as u32;
    let height = (size.height() * scale).ceil().clamp(1.0, 800.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or("Cannot allocate image")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
