// SPDX-License-Identifier: MIT

//! The window's 1 px frame and native interactive edge resize bands.

use rune::{El, ResizeEdge, TiledEdges, col, el};
use strata_kit::look::Look;

use super::chrome::WindowChrome;

/// Thickness of the side resize bands, in logical points.
const RESIZE_EDGE_THICKNESS: f32 = 6.0;
/// Size of each corner resize band, in logical points.
const RESIZE_CORNER_SIZE: f32 = 12.0;

pub fn frame(key: &str, content: El, look: &Look, chrome: &WindowChrome) -> El {
    let root = col().key(key.to_owned()).w_full().h_full().child(content);
    if !chrome.framed() {
        return root;
    }
    let line = look.kit.c.border_base;
    root.child(
        el().key(format!("{key}-line"))
            .pin(Some(0.0), Some(0.0), Some(0.0), None)
            .h(1.0)
            .fill(line)
            .inert()
            .a11y_hidden(),
    )
    .child(
        el().pin(Some(0.0), None, Some(0.0), Some(0.0))
            .h(1.0)
            .fill(line)
            .inert()
            .a11y_hidden(),
    )
    .child(
        el().pin(Some(0.0), Some(1.0), None, Some(1.0))
            .w(1.0)
            .fill(line)
            .inert()
            .a11y_hidden(),
    )
    .child(
        el().pin(None, Some(1.0), Some(0.0), Some(1.0))
            .w(1.0)
            .fill(line)
            .inert()
            .a11y_hidden(),
    )
}

/// Wraps `content` with interactive borderless resize bands for untiled window edges.
pub fn resize_bands(content: El, borderless: bool, chrome: &WindowChrome) -> El {
    let root = col()
        .key("window-resize-frame")
        .w_full()
        .h_full()
        .child(content);
    if !chrome.shows_resize_bands(borderless) {
        return root;
    }
    let tiled = chrome.tiled_edges.unwrap_or(TiledEdges::NONE);
    let edges = [
        (ResizeEdge::North, !tiled.top, "window-resize-n"),
        (
            ResizeEdge::NorthEast,
            !tiled.top && !tiled.right,
            "window-resize-ne",
        ),
        (ResizeEdge::East, !tiled.right, "window-resize-e"),
        (
            ResizeEdge::SouthEast,
            !tiled.bottom && !tiled.right,
            "window-resize-se",
        ),
        (ResizeEdge::South, !tiled.bottom, "window-resize-s"),
        (
            ResizeEdge::SouthWest,
            !tiled.bottom && !tiled.left,
            "window-resize-sw",
        ),
        (ResizeEdge::West, !tiled.left, "window-resize-w"),
        (
            ResizeEdge::NorthWest,
            !tiled.top && !tiled.left,
            "window-resize-nw",
        ),
    ];
    root.children(
        edges
            .into_iter()
            .filter(|&(_, active, _)| active)
            .map(|(edge, _, key)| resize_band(edge, key)),
    )
}

fn resize_band(edge: ResizeEdge, key: &'static str) -> El {
    let (t, c) = (RESIZE_EDGE_THICKNESS, RESIZE_CORNER_SIZE);
    let band = el().key(key).window_resize(edge).a11y_hidden();
    match edge {
        ResizeEdge::North => band.pin(Some(c), Some(0.0), Some(c), None).h(t),
        ResizeEdge::South => band.pin(Some(c), None, Some(c), Some(0.0)).h(t),
        ResizeEdge::West => band.pin(Some(0.0), Some(c), None, Some(c)).w(t),
        ResizeEdge::East => band.pin(None, Some(c), Some(0.0), Some(c)).w(t),
        ResizeEdge::NorthWest => band.pin(Some(0.0), Some(0.0), None, None).size(c, c),
        ResizeEdge::NorthEast => band.pin(None, Some(0.0), Some(0.0), None).size(c, c),
        ResizeEdge::SouthEast => band.pin(None, None, Some(0.0), Some(0.0)).size(c, c),
        ResizeEdge::SouthWest => band.pin(Some(0.0), None, None, Some(0.0)).size(c, c),
    }
}
