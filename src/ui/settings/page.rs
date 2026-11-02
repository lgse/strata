// SPDX-License-Identifier: MIT

//! Settings page scaffolding: the scrolling page container and the text
//! constraints that keep a page fitting its viewport.

use gtk::{glib, prelude::*, subclass::prelude::*};

// Reports no horizontal minimum, so the page scroller gives the page the viewport
// width instead of the content's own width. A page wider than the viewport would
// otherwise turn the scroller into a sideways pan over the settings.
mod page_content_bin {
    use super::*;

    #[derive(Default)]
    pub struct PageContentBin;

    #[glib::object_subclass]
    impl ObjectSubclass for PageContentBin {
        const NAME: &'static str = "StrataSettingsPageContentBin";
        type Type = super::PageContentBin;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for PageContentBin {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for PageContentBin {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            // The page height depends on the width, because description labels
            // wrap; without this the scroller caches a single-line height
            // measured before the viewport width is known and the page stops
            // short of its own content.
            self.obj()
                .first_child()
                .map_or(gtk::SizeRequestMode::ConstantSize, |child| {
                    child.request_mode()
                })
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(child) = self.obj().first_child() else {
                return (0, 0, -1, -1);
            };
            let (minimum, natural, minimum_baseline, natural_baseline) =
                child.measure(orientation, for_size);
            let minimum = if orientation == gtk::Orientation::Horizontal {
                0
            } else {
                minimum
            };
            (minimum, natural, minimum_baseline, natural_baseline)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.obj().first_child() {
                child.allocate(width, height, baseline, None);
            }
        }
    }
}

glib::wrapper! {
    pub struct PageContentBin(ObjectSubclass<page_content_bin::PageContentBin>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PageContentBin {
    fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let bin: Self = glib::Object::new();
        child.set_parent(&bin);
        bin
    }
}

pub(super) fn scrollable_page(content: &gtk::Box, class: Option<&str>) -> gtk::Widget {
    let empty = gtk::Label::new(Some(&crate::i18n::tr(
        "No matching settings are available on this installation.",
    )));
    empty.add_css_class("settings-search-page-empty");
    empty.add_css_class("settings-option-description");
    empty.set_visible(false);
    content.append(&empty);
    constrain_page_text(content.upcast_ref());
    content.set_hexpand(true);
    let page = PageContentBin::new(content);
    // GTK treats an `External` horizontal policy as scrollable, so sideways
    // swipes pan the page and paint the theme's near-black overshoot stripe.
    // `Never` refuses them; the bin keeps the page fitting the viewport, so
    // nothing is clipped.
    let scroller = gtk::ScrolledWindow::builder()
        .child(&page)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();
    scroller.add_css_class("settings-content-scroll");
    if let Some(class) = class {
        scroller.add_css_class(class);
    }
    scroller.upcast()
}

// Prose wraps to the viewport; long editable values scroll inside their entry,
// rather than making the whole settings page wider.
fn constrain_page_text(widget: &gtk::Widget) {
    // Action buttons keep native label sizing; segmented choices may wrap.
    if widget.is::<gtk::Button>() && !widget.is::<gtk::ToggleButton>() {
        return;
    }
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && label.ellipsize() == gtk::pango::EllipsizeMode::None
    {
        wrap_prose(label);
    }
    if let Some(entry) = widget.downcast_ref::<gtk::Entry>() {
        entry.set_width_chars(1);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        constrain_page_text(&widget);
    }
}

const NOWRAP_LABEL_CLASSES: [&str; 3] =
    ["settings-nowrap", "menu-heading", "settings-control-label"];

fn wrap_prose(label: &gtk::Label) {
    let wraps = !NOWRAP_LABEL_CLASSES
        .iter()
        .any(|class| label.has_css_class(class));
    label.set_wrap(wraps);
    label.set_wrap_mode(if label.has_css_class("settings-word-wrap") {
        gtk::pango::WrapMode::Word
    } else {
        gtk::pango::WrapMode::WordChar
    });
    crate::ui::controls::keep_words_whole(label);
}

pub(super) fn page_content() -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("settings-preferences");
    content
}
