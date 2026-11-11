//! Theme mapping: Strata's bundled themes, custom theme files and the Omarchy mapping onto
//! `rune_kit::Kit` and strata-kit's extensions.
//!
//! Work package: K1, `docs/work/phase-1-leaves/K1-theme-mapping.md`. Strata's behaviour is
//! read at lgse/strata `upstream/main` 35ee677ab: the plan's watermark (5738aad) isn't in
//! the clone, as for K2.
//!
//! # Use
//!
//! ```ignore
//! let library = Library::new(custom);               // custom: parse_custom(stem, &contents)?
//! let theme = match omarchy {                         // Strata's Follow Omarchy setting
//!     Some((name, colors)) => omarchy::theme(name, colors, variant),
//!     None => None,
//! };
//! let theme = theme.as_ref().unwrap_or_else(|| library.resolve(saved_id));
//! let next = theme.look().with_params(&params);      // then K4's overrides
//! strata_kit::theme::switch(&mut state.look, next, tx);
//! highlighter.set_theme(Arc::new(syntax::syntect_theme(&state.look))); // C10a re-colours
//! ```
//!
//! Nothing here reads a file. Strata reads the catalogue's replacements, the Omarchy state
//! and its own settings, watches them, and hands their contents in.
//!
//! - [`bundled`]: the 95 catalogue themes (`themes/catalog.toml`), with dim text lifted.
//! - [`parse_custom`] and [`Library`]: custom files, line-numbered errors, merging and the
//!   saved selection.
//! - [`omarchy::theme`]: the Quattro mapping and its variants.
//! - [`Theme::look`]: the mapping onto the look; its table is in `map.rs`.
//! - [`syntax`]: the source palette, [`syntax::SCOPES`], and [`syntax::syntect_theme`], the
//!   syntect theme C10a colours code with.
//! - [`switch`]: the animated move to the next look; at once with animations off.
//!
//! # Rulings recorded here
//!
//! - **Q8, scripts.** K1 implements no input behaviour, so it has no headless scripts. Its
//!   feature-map bullets are checked by the unit tests that name them, and the gallery's
//!   `tests/k1.rs` checks that a switch animates.
//! - **Q9, syntax.** Colour only: Strata's bold keywords and types and italic comments are
//!   dropped, functions, properties, parameters and punctuation draw in the text colour, and
//!   diff lines keep the kit's green and red (`syntax.rs`).
//! - **Q10, appearance.** `kit.dark` is `Kit::from_palette`'s. [`Theme::dark_background`] is
//!   Strata's own rule, for K3's Catppuccin flavour; the two agree on every bundled theme.
//! - **Q11, colour names.** CSS Color 4 names only; X11-only names GTK takes through Pango
//!   don't parse ([`Rgba8::parse`]).
//! - **Q12, goldens.** Three pairs: Catppuccin Latte and Mocha, Gruvbox light and dark hard,
//!   and Strata's own Omarchy Light and Azure Glow.
//! - **Q13, watermark.** 35ee677ab, above.
//! - **Q4, syntect.** [`syntax::syntect_theme`] (K1b) builds the syntect `Theme` from
//!   [`syntax::SCOPES`] and [`syntax::Role::color`]. strata-kit takes syntect with no
//!   features: the `highlighting` types only, no regex backend.
//! - **Q5, glow.** No rune-fx: the glow stays data on the look (`glow.color` is the accent),
//!   and components draw it, as C15 does.

mod catalog;
mod color;
mod colors;
mod file;
mod map;
pub mod omarchy;
mod switch;
pub mod syntax;

pub use catalog::{DEFAULT_ID, UNKNOWN_ID_FALLBACK, bundled, bundled_theme};
pub use color::Rgba8;
pub use colors::{
    Colors, DEFAULT_DANGER, ErrorKind, Key, Location, Source, SourceColors, Theme, ThemeError,
};
pub use file::{Library, parse_custom};
pub use switch::{MOTION, switch};
