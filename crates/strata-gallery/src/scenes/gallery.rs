//! The gallery's index page: the title, then one button per page, grouped by lane, which opens
//! it. Escape on a page comes back here.
//!
//! Each button is marked built or not built, from [`Page::implemented`], with a count and a
//! legend above the lanes. The golden scenes (`gallery-index-light`, `gallery-index-dark`) turn
//! the markers off, so the index's goldens list only what W0 declares (each page's lane, work
//! package and name) and don't change when a work package lands its first scenes. The review
//! scenes `gallery-index-status-light` and `-dark` show them, as the app does.

use rune::{Cx, El, Key, KeyEvent, Mods, Tx, col, el, row, text};
use rune_kit::{Button, Kit};

use strata_kit::look::Look;

use super::{Lane, PAGES, Page};
use crate::{Gallery, Scene};

/// The index's state.
pub struct Demo {
    /// Whether the index marks each page built or not built and shows the count and legend.
    /// True in the app and in `--snap` runs without a scene; the golden scenes turn it off so
    /// that landing a page's first scenes doesn't change the index goldens.
    pub markers: bool,
}

impl Default for Demo {
    fn default() -> Demo {
        Demo { markers: true }
    }
}

pub const SCENES: &[Scene] = &[
    ("gallery-index-light", |g| {
        (g.dark, g.look) = (false, Look::fallback(false));
        g.index.markers = false;
    }),
    ("gallery-index-dark", |g| {
        (g.dark, g.look) = (true, Look::fallback(true));
        g.index.markers = false;
    }),
];

pub const REVIEW: &[Scene] = &[
    ("gallery-index-status-light", |g| {
        (g.dark, g.look) = (false, Look::fallback(false))
    }),
    ("gallery-index-status-dark", |g| {
        (g.dark, g.look) = (true, Look::fallback(true))
    }),
];

/// The width of a column of buttons: room for the longest label and its marker.
const COLUMN_WIDTH: f32 = 296.0;

/// The column a lane's group sits in, chosen so the columns are about the same height.
fn column(lane: Lane) -> usize {
    match lane {
        Lane::Groundwork | Lane::Look | Lane::Collection => 0,
        Lane::Chrome => 1,
        Lane::Preview | Lane::Controls => 2,
    }
}

const COLUMNS: usize = 3;

/// What assistive technology hears after a page's name, and what the legend says.
fn status(implemented: bool) -> &'static str {
    if implemented {
        "Implemented"
    } else {
        "Not built yet"
    }
}

/// A status marker: a filled dot for built, a hollow ring for not built, so the shape says it
/// as well as the colour.
fn marker(kit: &Kit, implemented: bool) -> El {
    let dot = el().size(8.0, 8.0).radius(4.0).a11y_hidden();
    if implemented {
        dot.fill(kit.c.success.icon)
    } else {
        dot.border(1.5, kit.c.icon_faint)
    }
}

fn entry(kit: &Kit, markers: bool, p: &Page) -> El {
    let id = p.id;
    let label = format!("{} {}", p.wp, p.name);
    let shown = label.clone();
    let mark = markers.then(|| marker(kit, p.implemented()));
    let button = Button::new(label)
        .label_with(move |font, ink| {
            let text = text(shown).font(font).color(ink).grow(1.0);
            row()
                .grow(1.0)
                .items_center()
                .gap(8.0)
                .child(text)
                .children(mark)
        })
        .on_press(move |g: &mut Gallery| g.page = Some(id))
        .build(kit)
        .key(format!("index-{id}"))
        .w_full();
    if markers {
        button.description(status(p.implemented()))
    } else {
        button
    }
}

fn lane_group(g: &Gallery, lane: Lane) -> El {
    let kit = g.kit();
    let heading = text(lane.name())
        .role("heading")
        .level(2)
        .font(g.look.ty.heading)
        .color(kit.c.text_muted);
    let pages = PAGES.iter().filter(|p| p.lane == lane);
    col()
        .gap(4.0)
        .child(heading)
        .children(pages.map(|p| entry(kit, g.index.markers, p)))
}

/// The count of built pages, then the legend for the markers.
fn summary(kit: &Kit) -> El {
    let built = PAGES.iter().filter(|p| p.implemented()).count();
    let muted = |s: &'static str| text(s).font(kit.ty.ui_compact).color(kit.c.text_muted);
    let legend = |implemented: bool| {
        row()
            .items_center()
            .gap(6.0)
            .child(marker(kit, implemented))
            .child(muted(status(implemented)))
    };
    row()
        .key("index-summary")
        .items_center()
        .gap(16.0)
        .child(
            text(format!("{built} of {} implemented", PAGES.len()))
                .key("index-count")
                .font(kit.ty.ui_compact)
                .color(kit.c.text_base),
        )
        .child(legend(true))
        .child(legend(false))
}

pub fn view(g: &Gallery, _cx: &mut Cx<Gallery>) -> El {
    let kit = g.kit();
    let mut columns: [Vec<El>; COLUMNS] = Default::default();
    for lane in Lane::ALL {
        columns[column(lane)].push(lane_group(g, lane));
    }
    let columns = columns
        .into_iter()
        .map(|groups| col().w(COLUMN_WIDTH).gap(16.0).children(groups));
    let title = text("Strata components")
        .key("title")
        .role("heading")
        .level(1)
        .font(g.look.ty.title)
        .color(kit.c.text_base);
    col()
        .w_full()
        .h_full()
        .center()
        .gap(16.0)
        .child(title)
        .children(g.index.markers.then(|| summary(kit)))
        .child(
            row()
                .key("index")
                .role("navigation")
                .label("Components")
                .items_start()
                .gap(20.0)
                .children(columns),
        )
}

/// Return opens the focused page. Rune gives Return on a focused button to the window's default
/// button, as AppKit does, and only Space presses the button itself; the index has no default
/// button, so the key reaches this hook naming the focused button.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    if key.key != Key::Enter || key.mods != Mods::default() {
        return false;
    }
    let focused = tx.focused().and_then(|k| k.strip_prefix("index-"));
    match PAGES.iter().find(|p| Some(p.id) == focused) {
        Some(p) => {
            g.page = Some(p.id);
            true
        }
        None => false,
    }
}

pub fn poll(_g: &mut Gallery, _t: f64, _tx: &mut Tx) -> bool {
    false
}
