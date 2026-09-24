// SPDX-License-Identifier: MIT

use resvg::{tiny_skia, usvg};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) fn image(path: &Path, edge: u32) -> Result<Vec<u8>, String> {
    // An SVG resvg rejects still reaches the raster loaders, which decode some
    // dialects resvg does not.
    if let Some(source) = super::svg_source(path)
        && let Ok(rendered) = svg(&source, edge)
    {
        return Ok(rendered.png);
    }
    super::render_raw(path, edge as i32)
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
    svg(&source, 800).map(|rendered| rendered.png)
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
    svg(&result, 800).map(|rendered| rendered.png)
}

fn svg_options() -> usvg::Options<'static> {
    usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    }
}

pub(super) fn svg_dimensions(source: &str) -> Option<(i32, i32)> {
    let tree = usvg::Tree::from_str(source, &svg_options()).ok()?;
    let size = tree.size();
    Some((size.width().round() as i32, size.height().round() as i32))
}

pub(super) struct RenderedSvg {
    pub png: Vec<u8>,
    pub width: i32,
    pub height: i32,
}

pub(super) fn svg(source: &str, edge: u32) -> Result<RenderedSvg, String> {
    let mut options = svg_options();
    // System font discovery parses every installed face; only pay it when the
    // document actually has text to lay out.
    if source.contains("<text") {
        load_text_fonts(source, &mut options);
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
    }
    let tree = usvg::Tree::from_str(source, &options).map_err(|error| error.to_string())?;
    let size = tree.size();
    let edge = edge.clamp(1, 800) as f32;
    let scale = (edge / size.width().max(size.height())).min(1.0);
    let width = (size.width() * scale).ceil().clamp(1.0, edge) as u32;
    let height = (size.height() * scale).ceil().clamp(1.0, edge) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or("Cannot allocate image")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let png = pixmap.encode_png().map_err(|error| error.to_string())?;
    Ok(RenderedSvg {
        png,
        width: size.width().round() as i32,
        height: size.height().round() as i32,
    })
}

const FONT_FILE_LIMIT: usize = 32;
const FONT_LIST_LIMIT: u64 = 4 * 1024 * 1024;
const FONT_FAMILY_LIMIT: usize = 16;

// First real family to try per generic alias, in preference order.
const GENERIC_FAMILIES: &[&[&str]] = &[
    &[
        "DejaVu Sans",
        "Liberation Sans",
        "Noto Sans",
        "FreeSans",
        "Arial",
    ],
    &[
        "DejaVu Serif",
        "Liberation Serif",
        "Noto Serif",
        "FreeSerif",
        "Times New Roman",
    ],
    &[
        "DejaVu Sans Mono",
        "Liberation Mono",
        "Noto Sans Mono",
        "FreeMono",
        "Courier New",
    ],
];

// fontconfig's cache resolves family names to files in tens of milliseconds,
// while fontdb's scan parses every installed face. Non-ASCII documents keep the
// full scan so glyph fallback can reach every installed script; `&#…;`
// references can encode non-ASCII glyphs inside otherwise-ASCII sources.
fn needs_full_font_scan(source: &str) -> bool {
    !source.is_ascii() || source.contains("&#")
}

fn load_text_fonts(source: &str, options: &mut usvg::Options) {
    if !needs_full_font_scan(source)
        && let Some(paths) = resolve_font_files(source)
    {
        for path in paths {
            let _ = options.fontdb_mut().load_font_file(path);
        }
    }
    if options.fontdb.faces().next().is_none() {
        options.fontdb_mut().load_system_fonts();
    }
}

fn resolve_font_files(source: &str) -> Option<Vec<PathBuf>> {
    let output = super::bounded_output(
        Command::new("fc-list").args(["--format", "%{file}\t%{family}\n"]),
        FONT_LIST_LIMIT,
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let mut available: HashMap<String, PathBuf> = HashMap::new();
    for line in listing.lines() {
        let Some((file, families)) = line.split_once('\t') else {
            continue;
        };
        for family in families.split(',') {
            let family = family.trim().to_lowercase();
            if !family.is_empty() {
                available
                    .entry(family)
                    .or_insert_with(|| PathBuf::from(file));
            }
        }
    }
    if available.is_empty() {
        return None;
    }
    let mut wanted = svg_font_families(source);
    for slot in GENERIC_FAMILIES {
        if let Some(name) = slot
            .iter()
            .find(|name| available.contains_key(&name.to_lowercase()))
        {
            wanted.push((*name).to_owned());
        }
    }
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for name in wanted {
        if paths.len() >= FONT_FILE_LIMIT {
            break;
        }
        if let Some(path) = available.get(&name.to_lowercase())
            && seen.insert(path)
        {
            paths.push(path.clone());
        }
    }
    (!paths.is_empty()).then_some(paths)
}

/// `font-family` appears as an attribute (`font-family="A, B"`) and inside CSS
/// (`font-family: A`); both forms end at a quote, `;`, or tag boundary.
fn svg_font_families(source: &str) -> Vec<String> {
    let mut families = Vec::new();
    for (index, _) in source.match_indices("font-family") {
        let mut rest = source[index + "font-family".len()..]
            .trim_start_matches(|c: char| c == '=' || c == ':' || c.is_whitespace());
        loop {
            let (value, quoted_end) = match rest.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let inner = &rest[quote.len_utf8()..];
                    let end = inner.find(quote).unwrap_or(inner.len());
                    (&inner[..end], end + 2 * quote.len_utf8())
                }
                _ => {
                    let end = rest
                        .find(['"', '\'', ';', '<', '>', '/', '}', ')'])
                        .unwrap_or(rest.len());
                    (&rest[..end], end)
                }
            };
            for name in value.split(',') {
                let name = name.trim().trim_matches(['"', '\'']);
                if (1..=128).contains(&name.len()) {
                    families.push(name.to_owned());
                }
            }
            let after =
                rest[quoted_end.min(rest.len())..].trim_start_matches(|c: char| c.is_whitespace());
            if let Some(tail) = after.strip_prefix(',') {
                rest = tail;
                continue;
            }
            break;
        }
        if families.len() >= FONT_FAMILY_LIMIT {
            families.truncate(FONT_FAMILY_LIMIT);
            break;
        }
    }
    families
}

#[cfg(test)]
mod tests;
