// SPDX-License-Identifier: MIT

//! Viewers and archive preview (C14b), over the pixels viewport in `viewport` (C14a).
//!
//! Work package: C14b, `docs/rune/work/phase-3-views/C14b-viewers.md`. Plan §4 C14. Feature map nodes
//! `preview/preview-panel/model-views`, `.../routes`, `.../gcode` and
//! `operations/archives/preview`. Strata's code these follow is at `upstream/main` 35ee677ab.
//!
//! | Module | What |
//! | --- | --- |
//! | [`image`] | C14a's viewport with the image's name and dimensions |
//! | [`model`] | 3D models' views, camera presets; pictures saved in G-code, 3MF and FreeCAD |
//! | [`route`] | A route's picture, a marker that plays along it, and its elevation profile |
//! | [`archive`] | An archive's member tree on C1e, read-only, and its password prompt |
//! | [`layout`] | The player layout the model and route views share, which C13 reuses |
//! | [`viewport`] | C14a |
//!
//! Each takes its state by accessor and owns no I/O: renders, routes and member trees arrive as
//! data, and the host renders, parses and lists in its sandbox. Strata's words are the defaults
//! of each view's labels; Strata passes translated ones.
//!
//! # Deviations, for the owner
//!
//! - **No orbit by drag** (from the work package; coordinator ruling). The work package says
//!   "Orbit by drag sends a camera intent". Strata has none: "Only the two buttons change the
//!   view" and "The model picture keeps the grab cursor and file drag"
//!   (`preview/preview-panel/model-views`), and its design chose presets over free rotation
//!   because the renderer is a sandboxed software one. The model view follows Strata: a drag on
//!   the picture drags the file.
//! - **The elevation profile is drawn here** (from Strata; coordinator ruling, as the work
//!   package says). Strata's helper draws it into an 800 × 70 PNG in the theme's accent. Here it
//!   is a `canvas` shape from the samples, in the same space, from Kit tokens: a theme switch
//!   recolours it without a render, and it stays sharp at any width. The helper's PNG goes
//!   unused; the sandbox protocol is unchanged ([`route`]).
//! - **The image view's caption is new** (coordinator ruling): the name and "W × H" under the
//!   picture, and the dimensions as its accessible description. Strata shows the picture alone.
//! - **No 2× cap on enlarging** (coordinator ruling). Strata's media layout enlarges a picture
//!   at most twice; C14a's Fit has no cap, so a small photo fills the pane. An embedded model
//!   picture uses ScaleDown, never enlarged, as the G-code bullet asks, where Strata enlarges a
//!   3MF or FreeCAD thumbnail up to twice.
//! - **A route's descriptions change with its distance text** rather than every frame, which
//!   Strata's known gaps list as AT-SPI traffic.
//! - **G-code badges show while loading only**, as in Strata; the work package says "an image
//!   with its badges".
//! - **Badges aren't live regions**: rune-kit's `Badge` is one; Strata's are plain labels
//!   ([`layout`]).
//! - **Left at the archive's root is taken** when the host hears it
//!   ([`archive::ArchiveView::on_leave`]): Strata's tree passes it on, and its listing takes
//!   focus back. Here the host moves focus in
//!   `on_leave`; with no `on_leave`, the key goes on as in Strata.
//!
//! # Rune drafts (not filed)
//!
//! - **R-C14b-2: no wait cursor** (draft, not filed). Strata shows a wait cursor over the
//!   picture while a view renders. `rune::Cursor` has no wait or progress shape at Rune `631e8a67`
//!   (Default, Pointer, Text, Grab, Grabbing, NotAllowed, Move, the resize cursors, Crosshair,
//!   Cell, Hidden, Image), so the default arrow stands in. Repro: none needed; the enum lists
//!   them. Ask: `Cursor::Wait` and `Cursor::Progress`.
//!
//! Retired at Rune `631e8a67`: R-C14b-1 (`El::busy(bool)`, `#732`) and R-C14b-3
//! (`TextInput::description`, `#1015`). Earlier drafts these views meet: U-C1f-6 (`live_text`
//! turns off the virtual-window fast path: why the route's labels commit instead, [`route`]).

pub mod archive;
pub mod image;
pub mod layout;
pub mod model;
pub mod route;
pub mod viewport;
