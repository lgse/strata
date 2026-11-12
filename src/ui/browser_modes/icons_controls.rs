// SPDX-License-Identifier: MIT

//! The Icons header: navigation, actions, and the thumbnail size popover.

use std::rc::Rc;

use gtk::prelude::*;

use super::{
    MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE,
    filter::filter_controls,
    pane_header::{ContentActions, list_navigation},
};
use crate::{
    app::Browser,
    model::Location,
    ui::{browser, preferences::PreferenceManager},
};

pub(super) struct IconsControls {
    pub(super) sort_direction_button: gtk::Button,
    pub(super) leading: gtk::Box,
    pub(super) actions: gtk::Box,
    pub(super) filter_entry: gtk::Entry,
    pub(super) filter_revealer: gtk::Revealer,
    pub(super) filter_button: gtk::ToggleButton,
    pub(super) thumbnail: ThumbnailSizeMenu,
    pub(super) content: ContentActions,
}

pub(super) fn icons_controls(
    browser: &Rc<Browser>,
    depth: usize,
    thumbnail_size: i32,
) -> IconsControls {
    let leading = list_navigation(browser);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("icons-header-actions");
    let thumbnail = ThumbnailSizeMenu::new(browser, depth, thumbnail_size);
    let content = ContentActions::append(&actions, browser, depth);
    actions.append(&browser::pane_refresh_button(browser, depth));
    actions.append(&thumbnail.menu);
    let sort_direction_button = browser::column_sort_direction_toggle(browser, depth);
    actions.append(&sort_direction_button);
    actions.append(&browser::column_sort_menu(browser, depth));

    let (filter_entry, filter_revealer, filter_button) = filter_controls("Filter icons (%{keys})");
    actions.append(&filter_button);
    IconsControls {
        sort_direction_button,
        leading,
        actions,
        filter_entry,
        filter_revealer,
        filter_button,
        thumbnail,
        content,
    }
}

/// The header button whose popover sizes thumbnails.
pub(super) struct ThumbnailSizeMenu {
    pub(super) scale: gtk::Scale,
    pub(super) value: gtk::Label,
    pub(super) popover: gtk::Popover,
    menu: gtk::MenuButton,
}

impl ThumbnailSizeMenu {
    pub(super) fn set_visible(&self, visible: bool) {
        self.menu.set_visible(visible);
    }

    fn new(browser: &Rc<Browser>, depth: usize, thumbnail_size: i32) -> Self {
        let popover = gtk::Popover::new();
        popover.set_has_arrow(false);
        popover.add_css_class("icons-thumbnail-popover");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let (heading, title, value) = thumbnail_heading(thumbnail_size);
        let scale = thumbnail_scale(thumbnail_size);
        let extremes = thumbnail_extremes();
        crate::ui::accessibility::set_labelled_by(&scale, &[title.upcast_ref()]);
        content.append(&heading);
        content.append(&scale);
        content.append(&extremes);
        let size_actions = SizeActions {
            browser: Rc::downgrade(browser),
            depth,
            scale: scale.downgrade(),
        };
        size_actions.append(&content, &popover, &scale);
        popover.set_child(Some(&content));
        let menu = gtk::MenuButton::builder()
            .tooltip_text(crate::i18n::tr("Thumbnail size"))
            .popover(&popover)
            .build();
        crate::ui::accessibility::set_label(&menu, &crate::i18n::tr("Thumbnail size"));
        crate::ui::controls::pane_header_action(&menu);
        menu.add_css_class("icons-thumbnail-menu");
        menu.set_child(Some(&crate::assets::chrome_icon(
            crate::assets::icons::PICTURES,
        )));
        Self {
            scale,
            value,
            popover,
            menu,
        }
    }
}

fn thumbnail_heading(thumbnail_size: i32) -> (gtk::Box, gtk::Label, gtk::Label) {
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let title = gtk::Label::new(Some(&crate::i18n::tr("Thumbnail size")));
    title.add_css_class("icons-thumbnail-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    let value = gtk::Label::new(Some(&format!("{thumbnail_size} px")));
    value.add_css_class("icons-thumbnail-value");
    heading.append(&title);
    heading.append(&value);
    (heading, title, value)
}

fn thumbnail_scale(thumbnail_size: i32) -> gtk::Scale {
    let scale = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        f64::from(MIN_ICONS_THUMBNAIL_SIZE),
        f64::from(MAX_ICONS_THUMBNAIL_SIZE),
        16.0,
    );
    scale.set_increments(16.0, 1.0);
    scale.add_css_class("icons-thumbnail-scale");
    scale.set_draw_value(false);
    scale.set_value(f64::from(thumbnail_size));
    scale.set_size_request(220, -1);
    disable_scale_long_press_zoom(&scale);
    scale
}

fn thumbnail_extremes() -> gtk::Box {
    let extremes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    extremes.add_css_class("icons-thumbnail-extremes");
    let small = gtk::Label::new(Some(&crate::i18n::tr("Small")));
    small.set_xalign(0.0);
    small.set_hexpand(true);
    let large = gtk::Label::new(Some(&crate::i18n::tr("Large")));
    large.set_xalign(1.0);
    extremes.append(&small);
    extremes.append(&large);
    extremes
}

/// Make-default and reset actions for the shown folder's thumbnail size.
#[derive(Clone)]
struct SizeActions {
    browser: std::rc::Weak<Browser>,
    depth: usize,
    scale: gtk::glib::WeakRef<gtk::Scale>,
}

impl SizeActions {
    fn remembered_location(&self) -> Option<Location> {
        let location = self.browser.upgrade()?.location_at(self.depth)?;
        PreferenceManager::shared()
            .remembers(&location)
            .then_some(location)
    }

    fn make_default(&self) {
        if let Some(scale) = self.scale.upgrade() {
            PreferenceManager::shared().set_default_icons_size(scale.value().round() as i32);
        }
    }

    fn reset(&self) {
        if let Some(location) = self.remembered_location() {
            PreferenceManager::shared().reset_folder_icons_size(&location);
        }
    }

    /// Whether the remembered folder's size differs from the default.
    fn differs(&self, scale: &gtk::Scale) -> bool {
        self.remembered_location().is_some()
            && scale.value().round() as i32 != PreferenceManager::shared().icons_thumbnail_size()
    }

    /// Shown only when a remembered folder stores its own size, i.e. it differs from the default.
    fn append(&self, content: &gtk::Box, popover: &gtk::Popover, scale: &gtk::Scale) {
        let make_default = self.clone();
        let reset = self.clone();
        let actions = crate::ui::controls::DefaultActions::new(
            popover,
            (
                &crate::i18n::tr("Make this the default size"),
                &crate::i18n::tr("Reset to default size"),
            ),
            move || make_default.make_default(),
            move || reset.reset(),
        );
        actions.widget().add_css_class("column-menu");
        content.append(actions.widget());

        let sync = {
            let size = self.clone();
            move || {
                let Some(scale) = size.scale.upgrade() else {
                    return;
                };
                let differs = size.differs(&scale);
                actions.set_visible(differs, differs);
            }
        };
        sync();
        {
            let sync = sync.clone();
            popover.connect_map(move |_| sync());
        }
        scale.connect_value_changed(move |_| sync());
    }
}

pub(in crate::ui) fn disable_scale_long_press_zoom(scale: &gtk::Scale) {
    let controllers = scale.observe_controllers();
    let long_presses: Vec<gtk::GestureLongPress> = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)?
                .downcast::<gtk::GestureLongPress>()
                .ok()
        })
        .collect();
    for long_press in long_presses {
        scale.remove_controller(&long_press);
    }
}
