// SPDX-License-Identifier: MIT

use std::cell::Cell;

use gtk::{glib, graphene, gsk, prelude::*, subclass::prelude::*};

use super::artwork::ASPECT;

const PADDING: i32 = 16;
const GAP: i32 = 12;
const CONTROLS_GAP: i32 = 4;
const ART_MIN: i32 = 96;
const ART_MAX: i32 = 520;
const SPECTRUM_MIN: i32 = 44;
const SPECTRUM_MAX: i32 = 200;
const PANEL_MIN: i32 = 200;
const PANEL_MAX: i32 = 620;
/// Share of the stacked layout's free height the art may take before the spectrum.
const ART_SHARE: f32 = 0.62;
/// Share of the side-by-side layout's width the art (with its disc) may take.
const WIDE_ART_SHARE: f32 = 0.5;
// Hysteresis keeps a divider dragged near square from flipping the layout.
const ENTER_WIDE: f32 = 1.12;
const LEAVE_WIDE: f32 = 0.95;

/// Children in order: artwork, header, spectrum, scrubber, transport.
struct Parts {
    artwork: gtk::Widget,
    header: gtk::Widget,
    spectrum: gtk::Widget,
    scrubber: gtk::Widget,
    transport: gtk::Widget,
}

impl Parts {
    fn of(widget: &gtk::Widget) -> Option<Self> {
        let artwork = widget.first_child()?;
        let header = artwork.next_sibling()?;
        let spectrum = header.next_sibling()?;
        let scrubber = spectrum.next_sibling()?;
        let transport = scrubber.next_sibling()?;
        Some(Self {
            artwork,
            header,
            spectrum,
            scrubber,
            transport,
        })
    }

    fn panel_parts(&self) -> [&gtk::Widget; 3] {
        [&self.header, &self.scrubber, &self.transport]
    }

    /// The narrowest panel every control fits in.
    fn panel_minimum(&self) -> i32 {
        self.panel_parts()
            .iter()
            .map(|part| part.measure(gtk::Orientation::Horizontal, -1).0)
            .max()
            .unwrap_or(0)
            .max(PANEL_MIN)
    }

    fn height(widget: &gtk::Widget, width: i32) -> i32 {
        let width = width.max(widget.measure(gtk::Orientation::Horizontal, -1).0);
        widget.measure(gtk::Orientation::Vertical, width).1
    }

    fn fixed_height(&self, width: i32) -> i32 {
        Self::height(&self.header, width)
            + GAP
            + Self::height(&self.scrubber, width)
            + CONTROLS_GAP
            + Self::height(&self.transport, width)
    }
}

struct Plan {
    art: (i32, i32),
    panel_width: i32,
    spectrum: i32,
}

/// Sizes the art and spectrum for the space available. When room runs out the
/// spectrum shrinks and goes first, then the art.
fn plan(width: i32, height: i32, wide: bool, panel_min: i32, fixed: impl Fn(i32) -> i32) -> Plan {
    let art_width = |art_height: i32| (art_height as f32 * ASPECT).round() as i32;
    let content_height = (height - 2 * PADDING).max(0);
    if wide {
        let half = ((width - 3 * PADDING) as f32 * WIDE_ART_SHARE / ASPECT) as i32;
        let mut art_height = content_height.min(ART_MAX).min(half);
        let mut panel_width = width - 3 * PADDING - art_width(art_height);
        if panel_width < panel_min {
            art_height = ((width - 3 * PADDING - panel_min) as f32 / ASPECT) as i32;
            panel_width = panel_min;
        }
        if art_height < ART_MIN {
            art_height = 0;
            panel_width = width - 2 * PADDING;
        }
        let panel_width = panel_width.clamp(panel_min, PANEL_MAX.max(panel_min));
        let spectrum = (content_height - fixed(panel_width) - GAP).min(SPECTRUM_MAX);
        return Plan {
            art: (art_width(art_height), art_height),
            panel_width,
            spectrum: if spectrum < SPECTRUM_MIN { 0 } else { spectrum },
        };
    }
    let panel_width = (width - 2 * PADDING).clamp(panel_min, PANEL_MAX.max(panel_min));
    let free = content_height - fixed(panel_width) - GAP;
    let widest = ((width - 2 * PADDING) as f32 / ASPECT) as i32;
    let mut art_height = widest.min(ART_MAX).min((free as f32 * ART_SHARE) as i32);
    let mut spectrum = (free - art_height - GAP).min(SPECTRUM_MAX);
    if spectrum < SPECTRUM_MIN {
        spectrum = 0;
        art_height = widest.min(ART_MAX).min(free);
    }
    if art_height < ART_MIN {
        art_height = 0;
        spectrum = (content_height - fixed(panel_width) - GAP).min(SPECTRUM_MAX);
        if spectrum < SPECTRUM_MIN {
            spectrum = 0;
        }
    }
    Plan {
        art: (art_width(art_height), art_height),
        panel_width,
        spectrum,
    }
}

fn is_wide(width: i32, height: i32, was_wide: bool) -> bool {
    let ratio = width as f32 / height.max(1) as f32;
    if was_wide {
        ratio > LEAVE_WIDE
    } else {
        ratio >= ENTER_WIDE
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct NowPlayingLayout {
        pub(super) wide: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NowPlayingLayout {
        const NAME: &'static str = "StrataNowPlayingLayout";
        type Type = super::NowPlayingLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for NowPlayingLayout {}

    impl LayoutManagerImpl for NowPlayingLayout {
        fn request_mode(&self, _: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(
            &self,
            widget: &gtk::Widget,
            orientation: gtk::Orientation,
            _: i32,
        ) -> (i32, i32, i32, i32) {
            let Some(parts) = Parts::of(widget) else {
                return (0, 0, -1, -1);
            };
            let panel_min = parts.panel_minimum();
            if orientation == gtk::Orientation::Horizontal {
                let minimum = panel_min + 2 * PADDING;
                return (minimum, minimum, -1, -1);
            }
            let minimum = parts.fixed_height(panel_min) + 2 * PADDING;
            (minimum, minimum + SPECTRUM_MAX, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, _: i32) {
            let Some(parts) = Parts::of(widget) else {
                return;
            };
            let wide = is_wide(width, height, self.wide.get());
            self.wide.set(wide);
            let panel_min = parts.panel_minimum();
            let plan = plan(width, height, wide, panel_min, |width| {
                parts.fixed_height(width)
            });
            let (art_width, art_height) = plan.art;
            let fixed = parts.fixed_height(plan.panel_width);
            let panel_height = fixed
                + if plan.spectrum > 0 {
                    plan.spectrum + GAP
                } else {
                    0
                };
            let (art_x, art_y, x, mut y) = if wide {
                let group = plan.panel_width
                    + if art_width > 0 {
                        art_width + PADDING
                    } else {
                        0
                    };
                let left = (width - group) / 2;
                let panel_x = if art_width > 0 {
                    left + art_width + PADDING
                } else {
                    left
                };
                (
                    left,
                    (height - art_height) / 2,
                    panel_x,
                    (height - panel_height) / 2,
                )
            } else {
                let stack = panel_height + if art_height > 0 { art_height + GAP } else { 0 };
                let top = (height - stack) / 2;
                let panel_top = if art_height > 0 {
                    top + art_height + GAP
                } else {
                    top
                };
                (
                    (width - art_width) / 2,
                    top,
                    (width - plan.panel_width) / 2,
                    panel_top,
                )
            };
            allocate_at(&parts.artwork, art_x, art_y, art_width, art_height);
            let mut place = |part: &gtk::Widget, part_height: i32, gap: i32| {
                allocate_at(part, x, y, plan.panel_width, part_height);
                if part_height > 0 {
                    y += part_height + gap;
                }
            };
            let natural =
                |part: &gtk::Widget| part.measure(gtk::Orientation::Vertical, plan.panel_width).1;
            place(&parts.header, natural(&parts.header), GAP);
            place(&parts.spectrum, plan.spectrum, GAP);
            place(&parts.scrubber, natural(&parts.scrubber), CONTROLS_GAP);
            place(&parts.transport, natural(&parts.transport), 0);
        }
    }
}

fn allocate_at(widget: &gtk::Widget, x: i32, y: i32, width: i32, height: i32) {
    widget.set_child_visible(width > 0 && height > 0);
    widget.allocate(
        width.max(0),
        height.max(0),
        -1,
        Some(gsk::Transform::new().translate(&graphene::Point::new(x as f32, y as f32))),
    );
}

glib::wrapper! {
    pub struct NowPlayingLayout(ObjectSubclass<imp::NowPlayingLayout>)
        @extends gtk::LayoutManager;
}
