// SPDX-License-Identifier: MIT

use std::hash::{Hash, Hasher};

use rune::{Color, HelpLook, Icon, TextStyle};
use rune_kit::Kit;
use rune_kit::syntax::{Syntax, SyntaxTheme};

use crate::icons::{self, IconName, IconSet};
pub use crate::shape::LookParams;
use crate::slots::Slots;
use crate::transparency::{BaseSurfaces, Transparency};

#[cfg(test)]
mod tests;

pub fn fallback(dark: bool) -> Kit {
    Look::fallback(dark).kit
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Look {
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
    pub tiles: Ladder,
    pub thumbnails: ThumbnailSteps,
    pub roundness: f32,
    /// The element glow (Strata's "element glow" setting and its theme colour).
    pub glow: Glow,
    /// Theme fills Kit has no slot for.
    pub fill: Fills,
    pub syntax: SyntaxTheme,
    pub syntax_extra: SyntaxExtra,
    /// The icon catalog behind [`IconName`].
    pub icons: IconSet,
    pub font: Option<Families>,
    pub animations: bool,
    /// Resolved surface transparency and frosted glass parameters for this look.
    pub transparency: Transparency,
    /// Theme surface colors before transparency scaling, for idempotent [`Look::apply_params`].
    pub(crate) base_surfaces: BaseSurfaces,
    /// Component slots: tokens one component reads, which default to the tokens above.
    pub slots: Slots,
}

impl Look {
    pub fn fallback(dark: bool) -> Look {
        Look::from_kit(Kit::neutral(dark)).with_params(&LookParams::default())
    }

    pub fn from_kit(kit: Kit) -> Look {
        let syntax = SyntaxTheme::from_kit(&kit);
        let base_surfaces = BaseSurfaces::from_palette(&kit.c);
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
            transparency: Transparency::disabled(),
            base_surfaces,
            slots: Slots::default(),
            kit,
        }
    }

    /// The icon for `name` in the current catalog.
    pub fn icon(&self, name: IconName) -> &'static Icon {
        (self.icons.icon)(name)
    }

    pub fn icon_named(&self, key: &str) -> Option<&'static Icon> {
        (self.icons.named)(key)
    }

    pub fn round(&self, base: f32) -> f32 {
        crate::shape::scale_radius(base, self.roundness)
    }

    pub fn resize_caption(&self) -> Caption {
        Caption {
            fill: self.kit.c.bg_accent,
            ink: self.kit.c.text_inverse,
        }
    }

    pub fn ink_on(&self, fill: Color) -> Color {
        crate::theme::Rgba8::from_color(fill)
            .contrasting_ink()
            .color()
    }

    pub fn accent_icon_kit(&self) -> Kit {
        let mut kit = self.kit.clone();
        let accent = kit.c.icon_accent;
        kit.c.text_base = accent;
        kit.c.icon_base = accent;
        kit.c.icon_muted = accent;
        kit
    }

    pub fn help_look(&self) -> HelpLook {
        let kit = &self.kit;
        HelpLook::system(true)
            .fill(kit.alpha(kit.c.black, 0.8))
            .ink(kit.c.white)
            .ring(1.0, kit.light_on(0.1, kit.c.black))
            .radius(kit.radius.sm)
            .padding(10.0, 6.0)
            .font(kit.ty.ui_compact)
    }
}

/// A caption's fill and the colour of its text: [`Look::resize_caption`].
#[derive(Clone, Copy, Debug, PartialEq, Hash)]
#[non_exhaustive]
pub struct Caption {
    pub fill: Color,
    pub ink: Color,
}

/// The light fallback, so a state holding a `Look` can derive `Default`.
impl Default for Look {
    fn default() -> Look {
        Look::fallback(false)
    }
}

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
    pub inset: f32,
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
    pub pad: f32,
}

hash_lengths!(Content { pad });

#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RowHeights {
    pub list: f32,
    pub columns: f32,
    pub sidebar: f32,
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

#[derive(Clone, Copy, Debug, PartialEq, Hash)]
#[non_exhaustive]
pub struct Fills {
    pub muted: Color,
    pub highlight: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Hash)]
#[non_exhaustive]
pub struct SyntaxExtra {
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
    pub fn new(sizes: Vec<f32>) -> Option<ThumbnailSteps> {
        let increasing = sizes.windows(2).all(|w| w[0] < w[1]);
        let valid = sizes.iter().all(|s| s.is_finite() && *s > 0.0);
        (increasing && valid && !sizes.is_empty()).then_some(ThumbnailSteps { sizes })
    }

    pub fn sizes(&self) -> &[f32] {
        &self.sizes
    }

    pub fn for_size(&self, px: f32) -> usize {
        self.sizes
            .partition_point(|&s| s < px)
            .min(self.sizes.len() - 1)
    }
}
