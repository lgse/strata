//! `Look`: the `rune_kit::Kit` plus Strata's extension tokens, the one surface components read
//! their look from.
//!
//! Work package: W2, `docs/work/phase-0-groundwork/W2-look-contract.md`.
//!
//! # Building a look
//!
//! [`Look::fallback`] is the look before any Strata theme exists. The real one is built in
//! this order, each step changing the previous step's `Look` in place:
//!
//! 1. the theme (K1, `theme/`): [`Look::from_kit`] on the theme's kit, then `syntax`,
//!    `syntax_extra`, `fill` and `glow.color`;
//! 2. the user's shape, density and type settings (K2, `shape.rs`, `density.rs`,
//!    `typography.rs`): [`Look::apply_params`] sets `kit.radius` and `roundness`, `kit.ty`
//!    and `ty`, `frame`, `content`, `row`, `icon_size` and `font`, plus the two switches the
//!    look carries, `glow.on` and `animations`, so a look rebuilt from a theme keeps them;
//! 3. the user's overrides (K4, `overrides/`), by name through [`crate::tokens`];
//! 4. last, the syntect theme for C10a (K1), built from `syntax` and `syntax_extra` as they
//!    stand after the overrides, so an override of a `syntax.*` token reaches highlighted
//!    code. Nothing changes the syntax colours after this step without rebuilding it.
//!
//! The fallback is [`Look::from_kit`] on rune-kit's neutral kit, then step 2 at
//! [`LookParams::default`]: Strata's default roundness (60 %), Compact densities and 13 px
//! text. Its shape, density and type tokens are Strata's defaults, so a component built
//! against them keeps its sizes when K1 and K4 land; only the colours move.
//!
//! # Rules for components
//!
//! - Take the look as `&Look` in a builder, as rune-kit's builders take `&Kit`, and hand
//!   `&look.kit` to the rune-kit parts you use. Where a handler needs it later, take an
//!   accessor, `fn(&S) -> &Look`.
//! - Read fields, and a slot's own reader for a slot token (`slots::list::row_height`). Never
//!   call `tokens::get` while drawing: the registry is for override files, by name.
//! - Colours come from `kit.c` (or a slot token). Sizes and gaps come from `space`, `frame`,
//!   `row` and `icon_size`. Radii come from `kit.radius` or [`Look::round`]. Never write a
//!   literal colour, and never a literal size where a token says it.
//! - Don't clone a `Look` into a row closure or a memo. Copy the `Copy` token structs a row
//!   reads (`look.row`, `look.space`, `look.kit.c`, …); each is `Hash`, so it can be part of
//!   a `memo` version.
//! - Icons come from [`Look::icon`] by [`IconName`], or [`Look::icon_named`] for a catalog
//!   name (an entry's icon key). Tint them with Kit's icon roles; a multi-colour catalog's
//!   bodies name their own colours, and Rune draws those instead, so a component never needs
//!   to know which catalog is in use.
//! - Skip your own motion (flights, dissolves, open and close travel) when
//!   `look.animations` is false. The host also passes `!look.animations` to
//!   `Cx::reduced_motion` on every commit, so Rune's own motion follows.
//! - A component-specific token (a list row's height, say) is a slot: see [`crate::slots`].

use std::hash::{Hash, Hasher};

use rune::{Color, Icon, TextStyle};
use rune_kit::Kit;
use rune_kit::syntax::{Syntax, SyntaxTheme};

use crate::icons::{self, IconName, IconSet};
pub use crate::shape::LookParams;
use crate::slots::Slots;

#[cfg(test)]
mod tests;

/// The kit components draw with until a Strata theme is loaded: [`Look::fallback`]'s kit.
///
/// `lib.rs` re-exports this as `strata_kit::fallback`. Prefer [`Look::fallback`], which
/// carries Strata's extension tokens too.
pub fn fallback(dark: bool) -> Kit {
    Look::fallback(dark).kit
}

/// Everything a component reads its look from: rune-kit's tokens, plus the ones Strata adds.
///
/// Fields are public so the look's builders (K1, K2, K4) change them in place and components
/// read them. The struct is `non_exhaustive`: a new token is a new field, which breaks no
/// reader. Build one with [`Look::fallback`] or [`Look::from_kit`], never with a struct
/// literal.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Look {
    /// rune-kit's tokens: colours, elevation, glyphs, the type ramp, radii and motion. K2
    /// scales `kit.ty` by the text size and `kit.radius` by the roundness, so rune-kit's own
    /// components follow both.
    pub kit: Kit,
    /// The type styles Kit's ramp lacks. Scaled by the text size with `kit.ty`.
    pub ty: Ramp,
    /// Gaps and padding.
    pub space: Space,
    /// The window's bars: header, tab strip, sidebar rows, column and preview headers.
    pub frame: Frame,
    /// Spacing inside the file views.
    pub content: Content,
    /// Row height per view.
    pub row: RowHeights,
    /// Icon sizes.
    pub icon_size: IconSizes,
    /// The tile sizes IconsGrid steps through (C3): Ctrl+Shift+wheel and the size shortcuts.
    pub tiles: Ladder,
    /// The thumbnail size classes (C25): one rendered image per class, not per tile size.
    pub thumbnails: ThumbnailSteps,
    /// Corner roundness as a factor: 1.0 is 100 %. K2 has already applied it to
    /// `kit.radius`; [`Look::round`] applies it to a radius Kit doesn't have.
    pub roundness: f32,
    /// The element glow (Strata's "element glow" setting and its theme colour).
    pub glow: Glow,
    /// Theme fills Kit has no slot for.
    pub fill: Fills,
    /// Syntax colours for source previews and code blocks (C10a, C11), per rune-kit token
    /// kind. rune-kit's own highlighting reads this directly.
    pub syntax: SyntaxTheme,
    /// Syntax colours for kinds rune-kit's [`SyntaxTheme`] doesn't have. Only the syntect
    /// theme reads these.
    pub syntax_extra: SyntaxExtra,
    /// The icon catalog behind [`IconName`].
    pub icons: IconSet,
    /// The interface and code fonts. `None` keeps the app's own (`AppSpec::families`). The
    /// host passes a change to `Tx::set_font_families`; components never read this.
    pub font: Option<Families>,
    /// False when the user or the desktop turned animations off (P5's switch, as data). Set
    /// from [`LookParams::animations`], as `glow.on` is from [`LookParams::element_glow`].
    pub animations: bool,
    /// Component slots: tokens one component reads, which default to the tokens above.
    pub slots: Slots,
}

impl Look {
    /// Strata's look before a theme is loaded: rune-kit's neutral kit, with Strata's default
    /// shape, density and text size ([`LookParams::default`]).
    pub fn fallback(dark: bool) -> Look {
        // `apply_params` reads its base ramp from `from_kit`, never from here, so this
        // doesn't recurse.
        Look::from_kit(Kit::neutral(dark)).with_params(&LookParams::default())
    }

    /// `kit` with Strata's default extensions, the ones that follow a kit (the glow and
    /// syntax colours, the type ramp) derived from it. K1 starts each theme's look here. The
    /// shape, density and type tokens are the base step 2 starts from, not Strata's
    /// defaults: apply [`LookParams`] before drawing.
    pub fn from_kit(kit: Kit) -> Look {
        let syntax = SyntaxTheme::from_kit(&kit);
        Look {
            ty: Ramp::from_kit(&kit),
            space: Space {
                xxs: 2.0,
                xs: 4.0,
                sm: 6.0,
                md: 8.0,
                lg: 12.0,
                xl: 16.0,
                xxl: 24.0,
            },
            frame: Frame {
                inset: 0.0,
                bar: 40.0,
                control: 24.0,
            },
            content: Content { pad: 4.0 },
            row: RowHeights {
                list: 26.0,
                columns: 26.0,
                sidebar: 28.0,
                palette: 32.0,
            },
            icon_size: IconSizes {
                sm: 12.0,
                md: 16.0,
                lg: 20.0,
            },
            tiles: Ladder::strata_tiles(),
            thumbnails: ThumbnailSteps {
                sizes: vec![32.0, 64.0, 128.0, 256.0],
            },
            roundness: 1.0,
            glow: Glow {
                on: true,
                color: kit.c.text_accent,
                radius: 14.0,
                opacity: 0.24,
            },
            syntax_extra: SyntaxExtra {
                preprocessor: syntax.color(Syntax::Function),
            },
            fill: Fills {
                muted: kit.c.layer_02,
                highlight: kit.alpha(kit.c.bg_accent, 0.22),
            },
            syntax,
            icons: icons::lucide::SET,
            font: None,
            animations: true,
            slots: Slots::default(),
            kit,
        }
    }

    /// The icon for `name` in the current catalog.
    pub fn icon(&self, name: IconName) -> &'static Icon {
        (self.icons.icon)(name)
    }

    /// The icon a catalog calls `key`, such as an entry's icon key from Strata's file-name
    /// mapping, or `None` when the catalog has none by that name. Draw
    /// [`IconName::File`] then.
    pub fn icon_named(&self, key: &str) -> Option<&'static Icon> {
        (self.icons.named)(key)
    }

    /// `base`, a radius in points that Kit has no token for, at the current roundness,
    /// rounded to 0.01 pt as `kit.radius` is.
    pub fn round(&self, base: f32) -> f32 {
        crate::shape::scale_radius(base, self.roundness)
    }
}

/// The light fallback, so a state holding a `Look` can derive `Default`.
impl Default for Look {
    fn default() -> Look {
        Look::fallback(false)
    }
}

/// An `f32`'s bits for hashing, `-0.0` as `0.0` and every NaN alike, as rune-kit hashes
/// `Radius` and `Color`.
fn bits(x: f32) -> u32 {
    if x == 0.0 {
        0
    } else if x.is_nan() {
        f32::NAN.to_bits()
    } else {
        x.to_bits()
    }
}

/// `Hash` over the listed `f32` fields' [`bits`].
macro_rules! hash_lengths {
    ($ty:ident { $($field:ident),* $(,)? }) => {
        impl Hash for $ty {
            fn hash<H: Hasher>(&self, state: &mut H) {
                let $ty { $($field),* } = self;
                $(bits(*$field).hash(state);)*
            }
        }
    };
}

/// The type styles Strata adds to Kit's ramp (`kit.ty`).
#[derive(Clone, Copy, Debug, PartialEq, Hash)]
#[non_exhaustive]
pub struct Ramp {
    /// Dialog and panel titles.
    pub title: TextStyle,
    /// Section headings over rows (PINNED, DEVICES), and group headers.
    pub heading: TextStyle,
}

impl Ramp {
    fn from_kit(kit: &Kit) -> Ramp {
        Ramp {
            title: TextStyle {
                size: 15.0,
                weight: 600.0,
                ..kit.ty.ui
            }
            .lh(22.0),
            heading: kit.ty.small_medium.lh(16.0),
        }
    }
}

/// A spacing ramp in points, smallest first.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Space {
    pub xxs: f32,
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub xxl: f32,
}

hash_lengths!(Space {
    xxs,
    xs,
    sm,
    md,
    lg,
    xl,
    xxl
});

/// The window's bars, which Frame density grows together so they stay aligned.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Frame {
    /// Frame density: the extra padding above and below every bar's content, and before
    /// each bar action, in points. 0 is Strata's Compact.
    pub inset: f32,
    /// A bar's minimum height before the inset: the header, the tab strip and the preview
    /// header.
    pub bar: f32,
    /// A bar action's square size: header buttons, sidebar icons and device actions.
    pub control: f32,
}

hash_lengths!(Frame {
    inset,
    bar,
    control
});

/// Spacing inside the file views, which Content density sets.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Content {
    /// Content density: the space above and below a row's content (and a tile's), in
    /// points. 4 is Strata's Compact.
    pub pad: f32,
}

hash_lengths!(Content { pad });

/// Row heights per view, in points, at the current density and text size. K2 folds in
/// whichever density grows each view: Content density for the file views, Frame density for
/// the sidebar.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RowHeights {
    /// ListTable rows (C4).
    pub list: f32,
    /// ColumnsView rows (C2).
    pub columns: f32,
    /// PlacesSidebar rows (C8).
    pub sidebar: f32,
    /// Search palette result rows (C17). Folder peek (C22) reads `columns`: Strata's peek
    /// rows are `.file-row`s.
    pub palette: f32,
}

hash_lengths!(RowHeights {
    list,
    columns,
    sidebar,
    palette
});

/// Icon sizes in points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct IconSizes {
    /// Inline with small text: badges, trailing marks.
    pub sm: f32,
    /// Rows, bar actions and buttons.
    pub md: f32,
    /// Emphasis: empty states, dialog status icons.
    pub lg: f32,
}

hash_lengths!(IconSizes { sm, md, lg });

/// The element glow, as plain data. Where it is drawn, the component converts it to
/// rune-fx's glow parameters.
///
/// `radius` and `opacity` describe the standard glow, the one a focused or selected element
/// of ordinary size (a row, a button, a tab) gets: Strata's 14 px at 0.24 opacity. A larger
/// site (a dialog, a drop target over a whole view) scales `radius` from it in proportion
/// to its size and keeps `opacity`, so every glow follows an override of these two.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Glow {
    /// Strata's "element glow" setting.
    pub on: bool,
    /// The glow's colour, at full opacity.
    pub color: Color,
    /// The standard glow's reach, in points.
    pub radius: f32,
    /// The standard glow's opacity at its brightest, from 0 to 1.
    pub opacity: f32,
}

impl Hash for Glow {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let Glow {
            on,
            color,
            radius,
            opacity,
        } = self;
        on.hash(state);
        color.hash(state);
        bits(*radius).hash(state);
        bits(*opacity).hash(state);
    }
}

/// Fills a Strata theme sets that Kit has no slot for, named after Strata's theme roles so
/// a stylesheet's `@strata_muted` and `@strata_highlight` map onto them by name.
#[derive(Clone, Copy, Debug, PartialEq, Hash)]
#[non_exhaustive]
pub struct Fills {
    /// Strata's `muted`: the quiet control and hover surface (form controls, switch troughs,
    /// table headers). Defaults to `kit.c.layer_02`.
    pub muted: Color,
    /// Strata's `highlight`: the selection surface that drop targets, a selected search
    /// result and table header cells draw on. Defaults to the accent at 22 %.
    pub highlight: Color,
}

/// Syntax colours for the kinds Strata's themes colour and rune-kit's [`SyntaxTheme`] has
/// no kind for. Only the syntect theme (K1, C10a) reads these.
#[derive(Clone, Copy, Debug, PartialEq, Hash)]
#[non_exhaustive]
pub struct SyntaxExtra {
    /// Preprocessor directives and attributes (`#include`, `#[derive]`): Strata's
    /// `syntax_preprocessor`. Defaults to the function colour.
    pub preprocessor: Color,
}

/// The interface and code font families, as `AppSpec::families` takes them.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Families {
    pub ui: String,
    pub mono: String,
}

impl Families {
    pub fn new(ui: impl Into<String>, mono: impl Into<String>) -> Families {
        Families {
            ui: ui.into(),
            mono: mono.into(),
        }
    }
}

/// Sizes to step through, smallest first, with the size a reset returns to.
#[derive(Clone, Debug, PartialEq)]
pub struct Ladder {
    steps: Vec<f32>,
    default: f32,
}

impl Hash for Ladder {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.steps.len().hash(state);
        for &s in &self.steps {
            bits(s).hash(state);
        }
        bits(self.default).hash(state);
    }
}

impl Ladder {
    /// A ladder over `steps`, or `None` unless they are finite, positive and strictly
    /// increasing, with at least one step, and `default` is one of them.
    pub fn new(steps: Vec<f32>, default: f32) -> Option<Ladder> {
        let increasing = steps.windows(2).all(|w| w[0] < w[1]);
        let valid = steps.iter().all(|s| s.is_finite() && *s > 0.0);
        (increasing && valid && steps.contains(&default)).then_some(Ladder { steps, default })
    }

    /// Strata's tile sizes: finer where one step is a larger share of the size.
    fn strata_tiles() -> Ladder {
        let steps = [
            32.0, 36.0, 40.0, 44.0, 48.0, 56.0, 64.0, 72.0, 80.0, 96.0, 112.0, 128.0, 144.0, 160.0,
            192.0, 224.0, 256.0,
        ];
        Ladder {
            steps: steps.to_vec(),
            default: 64.0,
        }
    }

    pub fn steps(&self) -> &[f32] {
        &self.steps
    }

    pub fn default(&self) -> f32 {
        self.default
    }

    pub fn min(&self) -> f32 {
        self.steps[0]
    }

    pub fn max(&self) -> f32 {
        self.steps[self.steps.len() - 1]
    }

    /// `from` moved `delta` steps, clamped to the ladder's ends. From a size between two
    /// steps, the first step lands on the nearer one in the direction moved. A `delta` of 0
    /// returns `from` unchanged.
    pub fn step(&self, from: f32, delta: i32) -> f32 {
        let last = self.steps.len() as i64 - 1;
        let index = match delta.signum() {
            0 => return from,
            1 => self.steps.partition_point(|&s| s <= from) as i64 - 1 + i64::from(delta),
            _ => self.steps.partition_point(|&s| s < from) as i64 + i64::from(delta),
        };
        self.steps[index.clamp(0, last) as usize]
    }
}

/// Thumbnail size classes in pixels, smallest first. A view asks for the class that covers
/// its tile, so thumbnails are rendered once per class, not once per tile size.
///
/// Strata's are 32, 64, 128 and 256 px ([`Look::fallback`]). Rune draws a streamed `Pixels`
/// bilinearly with no mipmaps (C25's draft U-C25-1), which averages cleanly only down to 2:1,
/// so each class is twice the one below it: the class a draw gets is less than twice its size
/// for any draw of 16 px or more, where a 128 px image drawn at 37 px aliases.
///
/// There is no 512 px class for tiles over 128 pt at scale 2. Strata renders no thumbnail
/// larger than 256 px (`CANONICAL_MAX_EDGE`, `src/ui/thumbnail_cache.rs`), so a 512 step would
/// still get 256 px images, drawn larger as now; drawing larger softens but doesn't alias; and
/// a 512 px image holds 1 MiB (four times a 256 px one) of C25's budget, plus Rune's copy in
/// its image pages. Add one, with a larger budget, when Strata's canonical edge grows.
#[derive(Clone, Debug, PartialEq)]
pub struct ThumbnailSteps {
    sizes: Vec<f32>,
}

impl Hash for ThumbnailSteps {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.sizes.len().hash(state);
        for &s in &self.sizes {
            bits(s).hash(state);
        }
    }
}

impl ThumbnailSteps {
    /// Classes over `sizes`, or `None` unless they are finite, positive and strictly
    /// increasing, with at least one.
    pub fn new(sizes: Vec<f32>) -> Option<ThumbnailSteps> {
        let increasing = sizes.windows(2).all(|w| w[0] < w[1]);
        let valid = sizes.iter().all(|s| s.is_finite() && *s > 0.0);
        (increasing && valid && !sizes.is_empty()).then_some(ThumbnailSteps { sizes })
    }

    pub fn sizes(&self) -> &[f32] {
        &self.sizes
    }

    /// The class for a tile `px` device pixels across: the smallest class at least that
    /// large, else the largest.
    pub fn for_size(&self, px: f32) -> usize {
        self.sizes
            .partition_point(|&s| s < px)
            .min(self.sizes.len() - 1)
    }
}
