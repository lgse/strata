//! The catalog generator `build.rs` runs (`#[path]`-included there): SVG sources in, the Rust
//! source of [`super::catalog`]'s arrays out. It lives here so the crate's tests can check what
//! it accepts and what it makes; the library itself never compiles it.
//!
//! Each source becomes a `rune::Icon`: the elements inside its `<svg>`, with the root's paint
//! attributes moved to a group, as Rune's icon bodies have no `<svg>`. Three arrays hold every
//! icon, sorted by name, so a lookup by name is a binary search:
//!
//! - `LUCIDE`: Strata's Theme colors. Catppuccin colours become `currentColor`, with Strata's
//!   15 % fill on unfilled shapes, so they take the element's colour.
//! - `MOCHA` and `LATTE`: Strata's Brand colors, each Catppuccin colour resolved in that
//!   flavour. Rune draws a body that names its own colours in them.
//!
//! The Lucide and repository icons are the same in all three; Strata's tint placeholders in
//! them become `currentColor`.
//!
//! # What a source may hold
//!
//! A source outside these lists fails the build with its file's name, so a refresh can't
//! change what the catalogs draw without someone looking:
//!
//! - a name of lowercase ASCII letters, digits, `-` and `_`, starting with a letter;
//! - one root `<svg>` with a `0 0 s s` viewBox, `xmlns`, `width`, `height` and paint
//!   attributes ([`PAINT`]), and only whitespace and comments outside it: no XML
//!   declaration, doctype or trailing markup;
//! - the elements in [`ELEMENTS`], with the attributes in [`ATTRIBUTES`]: no `style`, no
//!   `class`, no `href`, no `<use>`, `<image>`, `<style>` or `<script>`;
//! - no text between tags (none of these elements holds any) and no `&`: no entity or
//!   character references, in text or in attribute values;
//! - paint (`fill`, `stroke`, `stop-color`) that is `none`, `currentColor`, a reference inside
//!   the file (`url(#id)`) and, by folder: Strata's tint placeholders in `lucide/` (and its
//!   white in `strata-check-on-primary`), or Catppuccin's `var(--vscode-ctp-<colour>)` in
//!   `catppuccin/`, plus the raw colours in [`CATPPUCCIN_OWN_COLOURS`]. No named colours.
//!
//! Rune's single-colour rasterizer ignores `stroke-dasharray`, `stroke-miterlimit` and
//! `clip-path`. They are allowed, as Brand colours draw them through resvg, and the
//! Theme-colours bodies that hold them are listed in `icons/README.md` (Rune draft U-K3-3).
//!
//! std only, deterministic: sorted output, no paths, times or hash order.

use std::fmt::Write as _;

// build.rs includes this file by `#[path]`, which would look for `tests.rs` beside it, so the
// path is spelt out: the same file from the crate and from build.rs (and rustfmt).
#[cfg(test)]
#[path = "generate/tests.rs"]
mod tests;

/// Which folder a source comes from, which decides its colours.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `lucide/`, `lucide-extra/` and `own/`: single-colour, tinted by the element.
    Lucide,
    /// `catppuccin/`: Catppuccin's colour variables.
    Catppuccin,
}

/// Strata's tint placeholders in its Lucide SVGs (`recolor_icon_source` in `src/assets.rs`).
const TINTS: [&str; 3] = ["#8bc9eb", "#22d3ee", "#2e3436"];

/// The one Lucide icon drawn in white: Strata's check on an accent fill. Its consumer tints
/// it with an on-accent colour, so here it takes `currentColor` like the rest.
const ON_ACCENT: (&str, &str) = ("strata-check-on-primary", "#ffffff");

/// Raw colours Strata's Catppuccin sources hold outside `var()`: a white clip path that
/// paints nothing, and a gradient's white stop. Both keep their colour in every catalog.
const CATPPUCCIN_OWN_COLOURS: [(&str, &str); 2] = [
    ("strata-ctp-humans", "#fff"),
    ("strata-ctp-poetry-lock", "#fff"),
];

/// Catppuccin Mocha and Latte, as Strata's `src/assets.rs` (`CATPPUCCIN_MOCHA`, `_LATTE`) has them.
const MOCHA: [(&str, &str); 16] = [
    ("rosewater", "#f5e0dc"),
    ("flamingo", "#f2cdcd"),
    ("pink", "#f5c2e7"),
    ("mauve", "#cba6f7"),
    ("red", "#f38ba8"),
    ("maroon", "#eba0ac"),
    ("peach", "#fab387"),
    ("yellow", "#f9e2af"),
    ("green", "#a6e3a1"),
    ("teal", "#94e2d5"),
    ("sky", "#89dceb"),
    ("sapphire", "#74c7ec"),
    ("blue", "#89b4fa"),
    ("lavender", "#b4befe"),
    ("text", "#cdd6f4"),
    ("overlay1", "#7f849c"),
];
const LATTE: [(&str, &str); 16] = [
    ("rosewater", "#dc8a78"),
    ("flamingo", "#dd7878"),
    ("pink", "#ea76cb"),
    ("mauve", "#8839ef"),
    ("red", "#d20f39"),
    ("maroon", "#e64553"),
    ("peach", "#fe640b"),
    ("yellow", "#df8e1d"),
    ("green", "#40a02b"),
    ("teal", "#179299"),
    ("sky", "#04a5e5"),
    ("sapphire", "#209fb5"),
    ("blue", "#1e66f5"),
    ("lavender", "#7287fd"),
    ("text", "#4c4f69"),
    ("overlay1", "#8c8fa1"),
];

/// Strata's fill for the folder and, under Theme colors, for a Catppuccin icon's unfilled
/// shapes: its own colour at 15 %.
const FAINT_FILL: &str = r#"fill="currentColor" fill-opacity="0.15""#;

/// Root attributes that paint, and move to the wrapping group.
const PAINT: [&str; 9] = [
    "fill",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "fill-rule",
    "opacity",
    "fill-opacity",
    "stroke-opacity",
];
/// Root attributes the body doesn't need.
const DROPPED: [&str; 4] = ["xmlns", "width", "height", "viewBox"];

/// Elements a body may hold: what Rune draws, and what resvg reads for the brand colours.
const ELEMENTS: [&str; 13] = [
    "g",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "defs",
    "clipPath",
    "linearGradient",
    "radialGradient",
    "stop",
];

/// Attributes an element may carry.
const ATTRIBUTES: [&str; 34] = [
    "d",
    "x",
    "y",
    "width",
    "height",
    "rx",
    "ry",
    "cx",
    "cy",
    "r",
    "x1",
    "y1",
    "x2",
    "y2",
    "points",
    "transform",
    "fill",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "fill-rule",
    "clip-rule",
    "opacity",
    "fill-opacity",
    "stroke-opacity",
    "stroke-dasharray",
    "stroke-miterlimit",
    "clip-path",
    "id",
    "offset",
    "stop-color",
    "gradientTransform",
    "gradientUnits",
];

/// Attributes that take a colour.
const COLOURS: [&str; 3] = ["fill", "stroke", "stop-color"];

/// One icon, ready for the catalogs.
pub struct Source {
    name: String,
    view: String,
    bodies: Bodies,
}

enum Bodies {
    /// The same in all three catalogs.
    Shared(String),
    Catppuccin {
        theme: String,
        mocha: String,
        latte: String,
    },
}

/// The icon `name` from `svg`, or why the build should stop.
pub fn icon(name: &str, svg: &str, kind: Kind) -> Result<Source, String> {
    check_name(name)?;
    let svg = strip_comments(svg);
    let (view, body) = parse(&svg)?;
    check_elements(name, &body, kind)?;

    let bodies = match kind {
        Kind::Catppuccin => Bodies::Catppuccin {
            theme: resolve(&body, None)?.replace(r#"fill="none""#, FAINT_FILL),
            mocha: resolve(&body, Some(&MOCHA))?,
            latte: resolve(&body, Some(&LATTE))?,
        },
        Kind::Lucide => {
            let mut body = body;
            for tint in TINTS {
                body = body.replace(tint, "currentColor");
            }
            if name == ON_ACCENT.0 {
                body = body.replace(ON_ACCENT.1, "currentColor");
            }
            if name == "strata-folder" {
                body = body.replacen(r#"fill="none""#, FAINT_FILL, 1);
            }
            Bodies::Shared(body)
        }
    };
    Ok(Source {
        name: name.to_owned(),
        view,
        bodies,
    })
}

fn check_name(name: &str) -> Result<(), String> {
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        return Err("a name starts with a lowercase letter (it becomes a Rust identifier)".into());
    }
    let allowed = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_';
    if !name.bytes().all(allowed) {
        return Err("names are lowercase ASCII letters, digits, `-` and `_`".into());
    }
    Ok(())
}

fn strip_comments(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        rest = rest[start..]
            .find("-->")
            .map_or("", |end| &rest[start + end + 3..]);
    }
    out.push_str(rest);
    out
}

/// The viewBox side, as written, and the body: the root's children, wrapped in a group with
/// its paint attributes.
fn parse(svg: &str) -> Result<(String, String), String> {
    let open = svg.find("<svg").ok_or("no <svg>")?;
    let head_end = open + svg[open..].find('>').ok_or("an unclosed <svg>")?;
    let close = svg.rfind("</svg>").ok_or("no </svg>")?;
    if close < head_end {
        return Err("an empty <svg>".into());
    }
    // Comments are already gone, so a prolog, a doctype (with its entities) or trailing
    // markup is all that could sit here.
    let outside = [&svg[..open], &svg[close + "</svg>".len()..]];
    if let Some(stray) = outside.iter().find(|s| !s.trim().is_empty()) {
        return Err(format!("`{}` outside the <svg>", stray.trim()));
    }
    if svg.contains('&') {
        return Err("an entity or character reference (`&`)".into());
    }
    let head = svg[open + 4..head_end].trim_end_matches('/');

    let mut view = None;
    let mut paint = Vec::new();
    for (key, value) in attributes(head)? {
        if key == "viewBox" {
            view = Some(square_view(value)?);
        } else if PAINT.contains(&key) {
            paint.push(format!(r#"{key}="{value}""#));
        } else if !DROPPED.contains(&key) {
            return Err(format!("an unknown root attribute `{key}`"));
        }
    }
    let view = view.ok_or("no viewBox")?;

    let inner = collapse(&svg[head_end + 1..close]);
    if inner.is_empty() {
        return Err("nothing to draw".into());
    }
    let body = if paint.is_empty() {
        inner
    } else {
        format!("<g {}>{inner}</g>", paint.join(" "))
    };
    Ok((view, body))
}

/// `name="value"` pairs in a tag's head.
fn attributes(head: &str) -> Result<Vec<(&str, &str)>, String> {
    let mut out = Vec::new();
    let mut rest = head.trim_start();
    while !rest.is_empty() {
        let eq = rest
            .find('=')
            .ok_or_else(|| format!("a stray `{rest}` in a tag"))?;
        let key = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let value = after
            .strip_prefix('"')
            .ok_or_else(|| format!("`{key}` isn't double-quoted"))?;
        let end = value
            .find('"')
            .ok_or_else(|| format!("`{key}` isn't closed"))?;
        out.push((key, &value[..end]));
        rest = value[end + 1..].trim_start();
    }
    Ok(out)
}

/// Every element in `body` is one Rune or resvg draws, with known attributes and colours.
fn check_elements(name: &str, body: &str, kind: Kind) -> Result<(), String> {
    let mut tags = body.split('<');
    // None of the elements Rune draws holds text, so text is a mistake or worse.
    let stray = |text: &str| match text.trim() {
        "" => Ok(()),
        text => Err(format!("stray text `{text}`")),
    };
    stray(tags.next().unwrap_or(""))?;
    for tag in tags {
        let (tag, after) = tag.split_once('>').ok_or("an unclosed tag")?;
        if !tag.starts_with('/') {
            check_element(name, tag, kind)?;
        }
        stray(after)?;
    }
    Ok(())
}

/// One opening tag, `<` and `>` taken off: an element and attributes the catalogs take.
fn check_element(name: &str, tag: &str, kind: Kind) -> Result<(), String> {
    let tag = tag.trim_end().trim_end_matches('/');
    let (element, head) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
    if !ELEMENTS.contains(&element) {
        return Err(format!("an element Rune doesn't draw: <{element}>"));
    }
    for (key, value) in attributes(head)? {
        if !ATTRIBUTES.contains(&key) {
            return Err(format!("an attribute the catalogs don't take: `{key}`"));
        }
        if key == "clip-path" {
            internal_reference(value)?;
        }
        if COLOURS.contains(&key) {
            colour(name, value, kind)?;
        }
    }
    Ok(())
}

/// A paint value this folder may use.
fn colour(name: &str, value: &str, kind: Kind) -> Result<(), String> {
    if value == "none" || value == "currentColor" {
        return Ok(());
    }
    if value.starts_with("url(") {
        return internal_reference(value);
    }
    let allowed = match kind {
        Kind::Lucide => TINTS.contains(&value) || (name, value) == ON_ACCENT,
        Kind::Catppuccin => {
            value.starts_with("var(") || CATPPUCCIN_OWN_COLOURS.contains(&(name, value))
        }
    };
    if allowed {
        Ok(())
    } else {
        Err(format!("a colour this folder doesn't take: `{value}`"))
    }
}

/// `url(#id)`: a reference to something in the same file, never outside it.
fn internal_reference(value: &str) -> Result<(), String> {
    let id = value
        .strip_prefix("url(#")
        .and_then(|v| v.strip_suffix(')'))
        .ok_or_else(|| format!("a reference outside the file: `{value}`"))?;
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(format!("a reference outside the file: `{value}`"));
    }
    Ok(())
}

/// The side of a `0 0 s s` viewBox, as written: Rune's icons are square and start at 0 0.
fn square_view(view: &str) -> Result<String, String> {
    let parts: Vec<&str> = view.split([' ', ',']).filter(|s| !s.is_empty()).collect();
    let numeric = |s: &str| {
        let digits = s.replace('.', "");
        !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && s.matches('.').count() <= 1
            && !s.starts_with('.')
            && !s.ends_with('.')
    };
    match parts.as_slice() {
        ["0", "0", w, h] if w == h && numeric(w) => Ok((*w).to_owned()),
        _ => Err(format!("viewBox `{view}` isn't `0 0 s s`")),
    }
}

/// One space for each run of whitespace, and none between tags.
fn collapse(s: &str) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.replace("> <", "><").trim().to_owned()
}

/// Catppuccin's `var(--vscode-ctp-<colour>)`s in `palette`, or as `currentColor` without one.
fn resolve(body: &str, palette: Option<&[(&str, &str); 16]>) -> Result<String, String> {
    const VAR: &str = "var(--vscode-ctp-";
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("var(") {
        out.push_str(&rest[..start]);
        let tail = rest[start..]
            .strip_prefix(VAR)
            .ok_or("a CSS variable that isn't Catppuccin's")?;
        let end = tail.find(')').ok_or("an unclosed var(")?;
        let colour = &tail[..end];
        let hex = MOCHA
            .iter()
            .position(|(c, _)| *c == colour)
            .ok_or_else(|| format!("`{colour}` isn't a Catppuccin colour"))?;
        out.push_str(palette.map_or("currentColor", |p| p[hex].1));
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn ident(name: &str) -> String {
    name.to_ascii_uppercase().replace('-', "_")
}

/// The catalogs' Rust source: index constants, then `LUCIDE`, `MOCHA` and `LATTE`.
pub fn catalog(mut sources: Vec<Source>) -> Result<String, String> {
    sources.sort_by(|a, b| a.name.cmp(&b.name));
    let mut idents: Vec<String> = Vec::with_capacity(sources.len());
    for (i, s) in sources.iter().enumerate() {
        if i > 0 && sources[i - 1].name == s.name {
            return Err(format!("{} is in two folders", s.name));
        }
        let id = ident(&s.name);
        if let Some(other) = idents.iter().position(|o| *o == id) {
            return Err(format!(
                "{} and {} share the identifier {id}",
                sources[other].name, s.name
            ));
        }
        idents.push(id);
    }

    let mut out = String::new();
    let w = &mut out;
    let _ = writeln!(
        w,
        "// Generated by strata-kit's build.rs from icons/. Do not edit.\n"
    );
    let _ = writeln!(
        w,
        "/// Each icon's index in [`LUCIDE`], [`MOCHA`] and [`LATTE`], by `Icon::name`."
    );
    let _ = writeln!(
        w,
        "#[expect(dead_code, reason = \"the semantic map uses only some of them\")]"
    );
    let _ = writeln!(w, "pub mod index {{");
    for (i, id) in idents.iter().enumerate() {
        let _ = writeln!(w, "    pub const {id}: usize = {i};");
    }
    let _ = writeln!(w, "}}\n");

    for (s, id) in sources.iter().zip(&idents) {
        if let Bodies::Shared(body) = &s.bodies {
            let _ = writeln!(w, "const B_{id}: &str = {body:?};");
        }
    }

    let n = sources.len();
    for (array, doc) in [
        (
            "LUCIDE",
            "Theme colors: every icon takes the element's colour.",
        ),
        ("MOCHA", "Brand colors, Catppuccin Mocha."),
        ("LATTE", "Brand colors, Catppuccin Latte."),
    ] {
        let _ = writeln!(w, "\n/// {doc} Sorted by name.");
        let _ = writeln!(w, "pub static {array}: [Icon; {n}] = [");
        for (s, id) in sources.iter().zip(&idents) {
            let body = match (&s.bodies, array) {
                (Bodies::Shared(_), _) => format!("B_{id}"),
                (Bodies::Catppuccin { theme, .. }, "LUCIDE") => format!("{theme:?}"),
                (Bodies::Catppuccin { mocha, .. }, "MOCHA") => format!("{mocha:?}"),
                (Bodies::Catppuccin { latte, .. }, _) => format!("{latte:?}"),
            };
            let _ = writeln!(
                w,
                "    Icon {{ name: {:?}, view: {}_f32, body: {body} }},",
                s.name, s.view
            );
        }
        let _ = writeln!(w, "];");
    }
    Ok(out)
}
