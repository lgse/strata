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
    let source = mermaid_svg::render(&source).map_err(|error| error.to_string())?;
    if source.len() > 2 * 1024 * 1024 {
        return Err("Mermaid output limit exceeded".into());
    }
    svg(&source)
}

pub(super) fn math(path: &Path, display: bool) -> Result<Vec<u8>, String> {
    let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
    if source.len() > crate::services::document_media::MATH_INPUT_LIMIT {
        return Err("Equation input limit exceeded".into());
    }
    let runtime = rquickjs::Runtime::new().map_err(|error| error.to_string())?;
    runtime.set_memory_limit(64 * 1024 * 1024);
    runtime.set_max_stack_size(1024 * 1024);
    let started = std::time::Instant::now();
    runtime.set_interrupt_handler(Some(Box::new(move || {
        started.elapsed() > std::time::Duration::from_secs(2)
    })));
    let context = rquickjs::Context::full(&runtime).map_err(|error| error.to_string())?;
    let result = context
        .with(|ctx| -> rquickjs::Result<String> {
            // Only our bundled program is evaluated. TeX is passed as a string argument,
            // with no host functions, module loader, filesystem, or network bindings.
            ctx.eval::<(), _>(include_str!("../../data/math-renderer.js"))?;
            let render: rquickjs::Function = ctx.globals().get("strataMath")?;
            render.call((source.as_str(), display))
        })
        .map_err(|_| "Invalid or unsupported LaTeX equation".to_owned())?;
    if result.len() > 2 * 1024 * 1024 {
        return Err("Equation output limit exceeded".into());
    }
    svg(&result)
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
    // Without fontconfig integration, fontdb's generic aliases can name missing
    // fonts. Resolve them to installed families before converting SVG text.
    let families: Vec<String> = options
        .fontdb
        .faces()
        .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
        .collect();
    let fallback = |preferred: &str| {
        families
            .iter()
            .find(|name| name.as_str() == preferred)
            .or_else(|| families.first())
            .cloned()
            .unwrap_or_else(|| preferred.to_owned())
    };
    let sans = fallback("DejaVu Sans");
    options.font_family = sans.clone();
    options.fontdb_mut().set_sans_serif_family(sans);
    options
        .fontdb_mut()
        .set_serif_family(fallback("DejaVu Serif"));
    options
        .fontdb_mut()
        .set_monospace_family(fallback("DejaVu Sans Mono"));
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
