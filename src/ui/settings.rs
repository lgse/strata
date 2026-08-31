// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::mpsc::TryRecvError,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*, subclass::prelude::*};

use crate::{
    assets::icons,
    services::{self, ReleaseMetadata, ReleaseNoteBlock, ReleaseNotes, UpdateCheck, UpdateInstall},
};

#[cfg(test)]
mod tests;

use super::{
    blur::BlurBin,
    browser::{BrowserView, dismiss_modal_layer, modal_layer},
    motion::set_reduce_motion,
    theme::{Theme, ThemeManager, ThemeTokens},
};

type ThemeCards = Rc<RefCell<Vec<(String, gtk::Button, gtk::Image)>>>;
pub(super) type UpdateNoticeHandler = Rc<dyn Fn(Option<(ReleaseMetadata, String)>)>;

const DIALOG_WIDTH: i32 = 920;
const DIALOG_HEIGHT: i32 = 620;
const DIALOG_MARGIN: i32 = 24;
const COMPACT_NAVIGATION_BREAKPOINT: i32 = 700;

mod responsive_bin {
    use super::*;

    #[derive(Default)]
    pub struct ResponsiveBin {
        pub compact_navigation: Cell<bool>,
        pub navigation: RefCell<Option<gtk::Box>>,
        pub navigation_heading: RefCell<Option<gtk::Label>>,
        pub navigation_labels: RefCell<Vec<gtk::Label>>,
        pub navigation_contents: RefCell<Vec<gtk::Box>>,
        pub responsive_flows: RefCell<Vec<(gtk::FlowBox, u32)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ResponsiveBin {
        const NAME: &'static str = "StrataSettingsResponsiveBin";
        type Type = super::ResponsiveBin;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ResponsiveBin {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for ResponsiveBin {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let natural = match orientation {
                gtk::Orientation::Horizontal => DIALOG_WIDTH + DIALOG_MARGIN * 2,
                gtk::Orientation::Vertical => DIALOG_HEIGHT + DIALOG_MARGIN * 2,
                _ => unreachable!("GTK orientations are horizontal or vertical"),
            };
            (1, natural, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let Some(child) = self.obj().first_child() else {
                return;
            };
            let (child_width, child_height) = responsive_dialog_size(width, height);
            let compact = uses_compact_navigation(child_width);
            if self.compact_navigation.replace(compact) != compact {
                if let Some(navigation) = self.navigation.borrow().as_ref() {
                    if compact {
                        navigation.add_css_class("compact");
                    } else {
                        navigation.remove_css_class("compact");
                    }
                }
                if let Some(heading) = self.navigation_heading.borrow().as_ref() {
                    heading.set_visible(!compact);
                }
                for label in self.navigation_labels.borrow().iter() {
                    label.set_visible(!compact);
                }
                for content in self.navigation_contents.borrow().iter() {
                    content.set_halign(if compact {
                        gtk::Align::Center
                    } else {
                        gtk::Align::Fill
                    });
                }
                for (flow, expanded_columns) in self.responsive_flows.borrow().iter() {
                    flow.set_max_children_per_line(if compact { 1 } else { *expanded_columns });
                }
            }
            let x = ((width - child_width) / 2) as f32;
            let y = ((height - child_height) / 2) as f32;
            let transform = gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x, y));
            child.allocate(child_width, child_height, baseline, Some(transform));
        }
    }
}

glib::wrapper! {
    pub struct ResponsiveBin(ObjectSubclass<responsive_bin::ResponsiveBin>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ResponsiveBin {
    fn new(
        child: &impl IsA<gtk::Widget>,
        navigation: &gtk::Box,
        navigation_heading: &gtk::Label,
        navigation_labels: Vec<gtk::Label>,
        navigation_contents: Vec<gtk::Box>,
        responsive_flows: Vec<(gtk::FlowBox, u32)>,
    ) -> Self {
        let bin: Self = glib::Object::new();
        let imp = bin.imp();
        imp.navigation.replace(Some(navigation.clone()));
        imp.navigation_heading
            .replace(Some(navigation_heading.clone()));
        imp.navigation_labels.replace(navigation_labels);
        imp.navigation_contents.replace(navigation_contents);
        imp.responsive_flows.replace(responsive_flows);
        child.set_parent(&bin);
        bin
    }
}

fn responsive_dialog_size(width: i32, height: i32) -> (i32, i32) {
    (
        DIALOG_WIDTH.min((width - DIALOG_MARGIN * 2).max(1)),
        DIALOG_HEIGHT.min((height - DIALOG_MARGIN * 2).max(1)),
    )
}

fn uses_compact_navigation(dialog_width: i32) -> bool {
    dialog_width < COMPACT_NAVIGATION_BREAKPOINT
}

pub fn build_layer(
    browser: &BrowserView,
    settings_button: &gtk::Button,
    root: &BlurBin,
    themes: Rc<ThemeManager>,
    update_notice: UpdateNoticeHandler,
) -> gtk::Box {
    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layer.add_css_class("settings-backdrop");
    layer.set_halign(gtk::Align::Fill);
    layer.set_valign(gtk::Align::Fill);
    layer.set_hexpand(true);
    layer.set_vexpand(true);
    layer.set_focusable(true);
    layer.set_visible(false);

    let panel = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    panel.add_css_class("settings-dialog");
    panel.set_overflow(gtk::Overflow::Hidden);

    let navigation = gtk::Box::new(gtk::Orientation::Vertical, 5);
    navigation.add_css_class("settings-navigation");
    let navigation_heading = append_heading(&navigation, "SETTINGS");

    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.add_css_class("settings-page");
    page.set_hexpand(true);
    let titlebar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    titlebar.add_css_class("settings-titlebar");
    let title = gtk::Label::new(Some("General"));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.add_css_class("settings-title");
    let close = gtk::Button::builder()
        .tooltip_text("Close settings")
        .build();
    close.set_child(Some(&crate::assets::primary_icon(icons::X, 18)));
    close.add_css_class("settings-close");
    close.set_valign(gtk::Align::Center);
    titlebar.append(&title);
    titlebar.append(&close);
    page.append(&titlebar);

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(120)
        .hhomogeneous(false)
        .vhomogeneous(false)
        .hexpand(true)
        .vexpand(true)
        .build();
    stack.add_named(&general_page(browser, themes.clone()), Some("general"));
    stack.add_named(
        &updates_page(themes.clone(), update_notice),
        Some("updates"),
    );
    stack.add_named(&keybindings_page(), Some("keybindings"));
    let (theme_page, responsive_flows) = theme_page(themes);
    stack.add_named(&theme_page, Some("theme"));
    stack.add_named(&about_page(), Some("about"));
    page.append(&stack);

    let nav_buttons: Rc<RefCell<Vec<(gtk::Button, gtk::Image, gtk::Image)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let mut navigation_labels = Vec::new();
    let mut navigation_contents = Vec::new();
    for (label, icon, name) in [
        ("General", icons::SLIDERS, "general"),
        ("Keybindings", icons::KEYBOARD, "keybindings"),
        ("Theme & appearance", icons::PALETTE, "theme"),
        ("Updates", icons::DOWNLOADS, "updates"),
        ("About", icons::INFO, "about"),
    ] {
        let active = name == "general";
        let (button, navigation_label, navigation_content, primary_icon, text_icon) =
            navigation_button(icon, label, active);
        navigation_labels.push(navigation_label);
        navigation_contents.push(navigation_content);
        if active {
            button.add_css_class("settings-nav-active");
        }
        nav_buttons
            .borrow_mut()
            .push((button.clone(), primary_icon, text_icon));
        let buttons = nav_buttons.clone();
        let stack = stack.clone();
        let title = title.clone();
        let page_title = label.to_owned();
        button.connect_clicked(move |clicked| {
            for (candidate, primary_icon, text_icon) in buttons.borrow().iter() {
                let active = candidate == clicked;
                if active {
                    candidate.add_css_class("settings-nav-active");
                } else {
                    candidate.remove_css_class("settings-nav-active");
                }
                primary_icon.set_visible(active);
                text_icon.set_visible(!active);
            }
            stack.set_visible_child_name(name);
            title.set_text(&page_title);
        });
        navigation.append(&button);
    }

    panel.append(&navigation);
    panel.append(&page);
    let responsive_panel = ResponsiveBin::new(
        &panel,
        &navigation,
        &navigation_heading,
        navigation_labels,
        navigation_contents,
        responsive_flows,
    );
    responsive_panel.set_hexpand(true);
    responsive_panel.set_vexpand(true);
    layer.append(&responsive_panel);

    let hidden_layer = layer.clone();
    let inactive_settings = settings_button.clone();
    let unblurred_root = root.clone();
    close.connect_clicked(move |_| hide(&hidden_layer, &inactive_settings, &unblurred_root));
    let hidden_layer = layer.clone();
    let inactive_settings = settings_button.clone();
    let unblurred_root = root.clone();
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key != gdk::Key::Escape {
            return gtk::glib::Propagation::Proceed;
        }
        hide(&hidden_layer, &inactive_settings, &unblurred_root);
        gtk::glib::Propagation::Stop
    });
    layer.add_controller(keys);
    layer
}

fn hide(layer: &gtk::Box, button: &gtk::Button, root: &BlurBin) {
    layer.set_visible(false);
    root.set_blurred(false);
    button.remove_css_class("active");
}

fn general_page(browser: &BrowserView, manager: Rc<ThemeManager>) -> gtk::Widget {
    let preferences = page_content();
    append_heading(&preferences, "BROWSING");
    let (peeking_row, peeking) = settings_option(
        "Folder peeking",
        "Preview folders automatically while moving through a pane.",
        true,
    );
    let browser_for_peeking = browser.clone();
    peeking.connect_active_notify(move |toggle| {
        browser_for_peeking.set_peek_enabled(toggle.is_active())
    });
    preferences.append(&peeking_row);

    let single_click_enabled = manager.single_click_previews();
    browser.set_single_click_previews(single_click_enabled);
    let (preview_row, single_click_previews) = settings_option(
        "Single-click file previews",
        "Show a quick preview when selecting a supported file.",
        single_click_enabled,
    );
    let browser_for_previews = browser.clone();
    let manager_for_previews = manager.clone();
    single_click_previews.connect_active_notify(move |toggle| {
        let enabled = toggle.is_active();
        browser_for_previews.set_single_click_previews(enabled);
        manager_for_previews.set_single_click_previews(enabled);
    });
    preferences.append(&preview_row);

    let direct_open_enabled = manager.search_open_files_directly();
    let (search_open_row, search_open_files) = settings_option(
        "Open search results directly",
        "Launch files from search instead of opening Strata's quick preview.",
        direct_open_enabled,
    );
    let manager_for_search_open = manager.clone();
    search_open_files.connect_active_notify(move |toggle| {
        manager_for_search_open.set_search_open_files_directly(toggle.is_active());
    });
    preferences.append(&search_open_row);

    append_heading(&preferences, "MOTION");
    let (motion_row, reduce_motion) = settings_option(
        "Reduce motion",
        "Disable nonessential interface animations.",
        false,
    );
    reduce_motion.connect_active_notify(|toggle| set_reduce_motion(toggle.is_active()));
    preferences.append(&motion_row);

    scrollable_page(&preferences, None)
}

fn updates_page(manager: Rc<ThemeManager>, update_notice: UpdateNoticeHandler) -> gtk::Widget {
    let preferences = page_content();
    append_heading(&preferences, "UPDATE PREFERENCES");
    let auto_check_enabled = manager.checks_for_updates();
    let (auto_check_row, auto_check) = settings_option(
        "Automatically check for updates",
        "Check GitHub for a newer release when Strata starts.",
        auto_check_enabled,
    );
    preferences.append(&auto_check_row);

    let available_notes = release_notes_card(
        "Available release",
        "Check for updates to see the latest release notes.",
    );
    let (update_row, run_check) = update_check_row(update_notice.clone(), available_notes.clone());
    preferences.append(&update_row);

    append_heading(&preferences, "RELEASE NOTES");
    let current_notes = release_notes_card(
        &format!("Current release · v{}", env!("CARGO_PKG_VERSION")),
        "Loading release notes…",
    );
    preferences.append(&current_notes.container);
    load_current_release_notes(&current_notes);

    let manager_for_updates = manager.clone();
    let toggled_check = run_check.clone();
    auto_check.connect_active_notify(move |toggle| {
        let enabled = toggle.is_active();
        manager_for_updates.set_checks_for_updates(enabled);
        if enabled {
            toggled_check();
        } else {
            update_notice(None);
        }
    });
    if auto_check_enabled {
        run_check();
    }

    scrollable_page(&preferences, None)
}

fn release_notes_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class("release-notes-content");
    label.set_xalign(0.0);
    label.set_yalign(0.0);
    label.set_hexpand(true);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_selectable(true);
    label.set_use_markup(true);
    label
}

fn clear_release_notes(notes: &gtk::Box) {
    while let Some(child) = notes.first_child() {
        notes.remove(&child);
    }
}

fn set_release_notes_message(notes: &gtk::Box, message: &str) {
    clear_release_notes(notes);
    let label = release_notes_label();
    label.set_text(message);
    notes.append(&label);
}

fn set_release_note_blocks(notes: &gtk::Box, blocks: &[ReleaseNoteBlock]) {
    clear_release_notes(notes);
    for block in blocks {
        match block {
            ReleaseNoteBlock::Heading { level, markup } => {
                let label = release_notes_label();
                label.add_css_class("release-notes-heading");
                label.add_css_class(&format!("level-{level}"));
                label.set_markup(markup);
                notes.append(&label);
            }
            ReleaseNoteBlock::Paragraph(markup) => {
                let label = release_notes_label();
                label.set_markup(markup);
                notes.append(&label);
            }
            ReleaseNoteBlock::ListItem {
                marker,
                depth,
                markup,
            } => {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row.set_valign(gtk::Align::Start);
                row.set_margin_start(i32::try_from(depth.saturating_mul(18)).unwrap_or(i32::MAX));
                let bullet = gtk::Label::new(Some(marker));
                bullet.add_css_class("release-notes-bullet");
                bullet.set_valign(gtk::Align::Start);
                let copy = release_notes_label();
                copy.set_markup(markup);
                row.append(&bullet);
                row.append(&copy);
                notes.append(&row);
            }
            ReleaseNoteBlock::Code(markup) => {
                let label = release_notes_label();
                label.add_css_class("release-notes-code");
                label.set_markup(&format!("<tt>{markup}</tt>"));
                notes.append(&label);
            }
            ReleaseNoteBlock::Rule => {
                let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
                separator.add_css_class("release-notes-rule");
                notes.append(&separator);
            }
        }
    }
}

#[derive(Clone)]
struct ReleaseNotesCard {
    container: gtk::Box,
    title: gtk::Label,
    notes: gtk::Box,
    fallback: gtk::LinkButton,
}

fn release_notes_card(title: &str, initial: &str) -> ReleaseNotesCard {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 8);
    container.add_css_class("release-notes-card");
    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("release-notes-title");
    title_label.set_xalign(0.0);
    title_label.set_wrap(true);
    let notes = gtk::Box::new(gtk::Orientation::Vertical, 6);
    set_release_notes_message(&notes, initial);
    let fallback =
        gtk::LinkButton::with_label("https://github.com/lgse/strata/releases", "View on GitHub");
    fallback.add_css_class("release-notes-fallback");
    fallback.set_halign(gtk::Align::Start);
    fallback.set_visible(false);
    container.append(&title_label);
    container.append(&notes);
    container.append(&fallback);
    ReleaseNotesCard {
        container,
        title: title_label,
        notes,
        fallback,
    }
}

fn show_release_notes(card: &ReleaseNotesCard, release: &ReleaseMetadata) {
    card.container.set_visible(true);
    card.title.set_text(&format!(
        "{} · v{}",
        card.title
            .text()
            .split('·')
            .next()
            .unwrap_or("Release")
            .trim(),
        release.version
    ));
    if release.notes.trim().is_empty() {
        set_release_notes_message(
            &card.notes,
            "No release notes were provided for this release.",
        );
    } else {
        set_release_note_blocks(&card.notes, &release.note_blocks);
    }
    card.fallback.set_uri(&release.url);
    card.fallback.set_visible(true);
}

fn load_current_release_notes(card: &ReleaseNotesCard) {
    let receiver = services::fetch_release_notes(env!("CARGO_PKG_VERSION"));
    let card = card.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        match receiver.try_recv() {
            Ok(ReleaseNotes::Found(release)) => {
                show_release_notes(&card, &release);
                glib::ControlFlow::Break
            }
            Ok(ReleaseNotes::Unavailable { url }) => {
                set_release_notes_message(
                    &card.notes,
                    "Release notes are unavailable because this version’s tag was not found.",
                );
                card.fallback.set_uri(&url);
                card.fallback.set_visible(true);
                glib::ControlFlow::Break
            }
            Ok(ReleaseNotes::Failed { message, url }) => {
                set_release_notes_message(
                    &card.notes,
                    &format!("Couldn’t load release notes: {message}"),
                );
                card.fallback.set_uri(&url);
                card.fallback.set_visible(true);
                glib::ControlFlow::Break
            }
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => {
                set_release_notes_message(
                    &card.notes,
                    "Couldn’t load release notes because the request ended unexpectedly.",
                );
                glib::ControlFlow::Break
            }
        }
    });
}

fn update_check_row(
    update_notice: UpdateNoticeHandler,
    available_notes: ReleaseNotesCard,
) -> (gtk::Box, Rc<dyn Fn()>) {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    row.add_css_class("settings-option");
    let summary = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    summary.set_vexpand(true);
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some("Check for updates"));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let status = gtk::Label::new(Some(&format!("Version {}", env!("CARGO_PKG_VERSION"))));
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.set_use_markup(true);
    status.add_css_class("settings-option-description");
    copy.append(&title);
    copy.append(&status);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("settings-update-progress");
    progress.set_hexpand(true);
    progress.set_visible(false);
    copy.append(&progress);
    let button = gtk::Button::with_label("Check now");
    button.add_css_class("settings-update-check");
    button.set_valign(gtk::Align::Center);
    summary.append(&copy);
    summary.append(&button);
    row.append(&summary);
    available_notes.container.add_css_class("inline");
    available_notes.container.set_visible(false);
    row.append(&available_notes.container);

    let checking = Rc::new(Cell::new(false));
    // Set once a check finds an update this platform can install; consumed by the
    // button's next click instead of re-running a check.
    let pending_download = Rc::new(RefCell::new(None::<String>));
    // Set once an install finishes, so the next click restarts instead of re-checking.
    let installed = Rc::new(Cell::new(false));

    let run_check: Rc<dyn Fn()> = Rc::new({
        let checking = checking.clone();
        let status = status.clone();
        let button = button.clone();
        let update_notice = update_notice.clone();
        let pending_download = pending_download.clone();
        let installed = installed.clone();
        let progress = progress.clone();
        let available_notes = available_notes.clone();
        move || {
            if checking.replace(true) {
                return;
            }
            *pending_download.borrow_mut() = None;
            installed.set(false);
            button.set_label("Check now");
            progress.set_fraction(0.0);
            progress.set_visible(false);
            progress.remove_css_class("error");
            status.set_text("Checking for updates…");
            available_notes.container.set_visible(false);
            available_notes.fallback.set_visible(false);
            button.set_sensitive(false);
            let receiver = services::check_for_updates(env!("CARGO_PKG_VERSION"));
            let checking = checking.clone();
            let status = status.clone();
            let button = button.clone();
            let update_notice = update_notice.clone();
            let pending_download = pending_download.clone();
            let available_notes = available_notes.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                match receiver.try_recv() {
                    Ok(result) => {
                        status.set_markup(&update_check_message(&result));
                        available_notes
                            .container
                            .set_visible(shows_available_release_notes(&result));
                        match &result {
                            UpdateCheck::Available {
                                release,
                                download_url: Some(download_url),
                            } => {
                                update_notice(Some((release.clone(), download_url.clone())));
                            }
                            UpdateCheck::UpToDate
                            | UpdateCheck::Available {
                                download_url: None, ..
                            } => update_notice(None),
                            UpdateCheck::Failed(_) => {}
                        }
                        match &result {
                            UpdateCheck::Available {
                                release,
                                download_url,
                            } => {
                                show_release_notes(&available_notes, release);
                                if let Some(download_url) = download_url {
                                    *pending_download.borrow_mut() = Some(download_url.clone());
                                    button.set_label("Install update");
                                }
                            }
                            UpdateCheck::UpToDate | UpdateCheck::Failed(_) => {}
                        }
                        button.set_sensitive(true);
                        checking.set(false);
                        glib::ControlFlow::Break
                    }
                    Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(TryRecvError::Disconnected) => {
                        status.set_markup(
                            "Couldn't check for updates · <a href=\"https://github.com/lgse/strata/releases/latest\">View releases on GitHub</a>",
                        );
                        available_notes.container.set_visible(false);
                        button.set_sensitive(true);
                        checking.set(false);
                        glib::ControlFlow::Break
                    }
                }
            });
        }
    });

    let clicked_check = run_check.clone();
    button.connect_clicked(move |button| {
        if installed.get() {
            restart_application(button);
            return;
        }
        if let Some(download_url) = pending_download.borrow_mut().take() {
            if checking.replace(true) {
                return;
            }
            status.set_text("Downloading update…");
            progress.set_fraction(0.0);
            progress.set_visible(true);
            progress.remove_css_class("error");
            button.set_sensitive(false);
            let receiver = services::install_update(download_url);
            let checking = checking.clone();
            let status = status.clone();
            let button = button.clone();
            let installed = installed.clone();
            let progress = progress.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                loop {
                    match receiver.try_recv() {
                        Ok(UpdateInstall::Downloading { downloaded, total }) => {
                            if let Some(total) = total.filter(|total| *total > 0) {
                                let fraction = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
                                progress.set_fraction(fraction);
                                status.set_text(&format!(
                                    "Downloading update… {:.0}%",
                                    fraction * 100.0
                                ));
                            } else {
                                progress.pulse();
                                status.set_text(&format!(
                                    "Downloading update… {:.1} MB",
                                    downloaded as f64 / 1_048_576.0
                                ));
                            }
                        }
                        Ok(UpdateInstall::Installing) => {
                            progress.set_fraction(1.0);
                            status.set_text("Verifying and installing update…");
                        }
                        Ok(UpdateInstall::Installed) => {
                            status.set_text("Update installed — restart to apply");
                            button.set_label("Restart now");
                            button.set_sensitive(true);
                            installed.set(true);
                            checking.set(false);
                            return glib::ControlFlow::Break;
                        }
                        Ok(UpdateInstall::Failed(message)) => {
                            status.set_text(&format!("Couldn't install update: {message}"));
                            progress.add_css_class("error");
                            button.set_label("Check now");
                            button.set_sensitive(true);
                            checking.set(false);
                            return glib::ControlFlow::Break;
                        }
                        Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
                        Err(TryRecvError::Disconnected) => {
                            status.set_text("Couldn't install update");
                            progress.add_css_class("error");
                            button.set_label("Check now");
                            button.set_sensitive(true);
                            checking.set(false);
                            return glib::ControlFlow::Break;
                        }
                    }
                }
            });
        } else {
            clicked_check();
        }
    });
    (row, run_check)
}

/// Relaunches the (just-updated) executable and quits the current instance.
fn restart_application(button: &gtk::Button) {
    let application = button
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok())
        .and_then(|window| window.application());
    restart(application.as_ref());
}

fn restart(application: Option<&gtk::Application>) {
    let Ok(mut current_exe) = std::env::current_exe() else {
        return;
    };
    // On Linux, replacing the running executable makes /proc/self/exe resolve to
    // the old path with " (deleted)" appended. Relaunch the replacement at the
    // original path instead of treating that suffix as part of the filename.
    if !current_exe.exists()
        && let Some(path) = current_exe
            .to_str()
            .and_then(|path| path.strip_suffix(" (deleted)"))
        && std::path::Path::new(path).is_file()
    {
        current_exe = path.into();
    }
    // Give GApplication time to release its single-instance bus name before the
    // replacement starts. Starting it immediately only re-activates this process.
    if std::process::Command::new("sh")
        .args(["-c", "sleep 0.25; exec \"$1\"", "strata-restart"])
        .arg(current_exe)
        .spawn()
        .is_err()
    {
        return;
    }
    match application {
        Some(application) => application.quit(),
        None => std::process::exit(0),
    }
}

pub(super) fn show_update_dialog(
    parent: &gtk::Window,
    release: &ReleaseMetadata,
    download_url: String,
) {
    let Some(window_overlay) = parent.child().and_downcast::<gtk::Overlay>() else {
        return;
    };
    let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
    if let Some(root) = blurred_root.as_ref() {
        root.set_blurred(true);
    }

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("transfer-dialog");
    content.add_css_class("update-dialog");
    content.set_halign(gtk::Align::Center);
    content.set_valign(gtk::Align::Center);
    content.set_size_request(560, -1);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.add_css_class("update-dialog-header");
    let symbol = gtk::CenterBox::new();
    symbol.add_css_class("update-dialog-symbol");
    symbol.set_size_request(40, 40);
    symbol.set_valign(gtk::Align::Center);
    symbol.set_center_widget(Some(&crate::assets::primary_icon(icons::DOWNLOADS, 20)));
    let heading = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let title = gtk::Label::new(Some(&format!("Strata v{} is available", release.version)));
    title.add_css_class("update-dialog-title");
    title.set_xalign(0.0);
    let subtitle = gtk::Label::new(Some(&format!(
        "Installed v{}  →  Available v{}",
        env!("CARGO_PKG_VERSION"),
        release.version
    )));
    subtitle.add_css_class("update-dialog-subtitle");
    subtitle.set_xalign(0.0);
    subtitle.set_wrap(true);
    heading.append(&title);
    heading.append(&subtitle);
    header.append(&symbol);
    header.append(&heading);
    content.append(&header);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.add_css_class("update-dialog-body");
    let notes_heading = gtk::Label::new(Some("What’s new"));
    notes_heading.add_css_class("release-notes-title");
    notes_heading.set_xalign(0.0);
    let notes = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if release.notes.trim().is_empty() {
        set_release_notes_message(
            &notes,
            "No release notes were provided. Review this release on GitHub before continuing.",
        );
    } else {
        set_release_note_blocks(&notes, &release.note_blocks);
    }
    let notes_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(120)
        .max_content_height(300)
        .propagate_natural_height(true)
        .child(&notes)
        .build();
    notes_scroll.add_css_class("update-dialog-notes");
    let fallback = gtk::LinkButton::with_label(&release.url, "View release on GitHub");
    fallback.add_css_class("release-notes-fallback");
    fallback.set_halign(gtk::Align::Start);
    let status = gtk::Label::new(Some(
        "Review the release notes before downloading the update.",
    ));
    status.add_css_class("update-dialog-status");
    status.set_xalign(0.0);
    status.set_wrap(true);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("update-dialog-progress");
    progress.set_fraction(0.0);
    progress.set_visible(false);
    body.append(&notes_heading);
    body.append(&notes_scroll);
    body.append(&fallback);
    body.append(&status);
    body.append(&progress);
    content.append(&body);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.add_css_class("update-dialog-actions");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("transfer-cancel");
    let action = gtk::Button::with_label("Download update");
    action.add_css_class("update-dialog-action");
    actions.append(&spacer);
    actions.append(&cancel);
    actions.append(&action);
    content.append(&actions);

    let layer = modal_layer(&content);
    window_overlay.add_overlay(&layer);
    action.grab_focus();

    let cancel_layer = layer.clone();
    let cancel_overlay = window_overlay.clone();
    let cancel_root = blurred_root.clone();
    cancel.connect_clicked(move |_| {
        dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
    });

    let started = Rc::new(Cell::new(false));
    let installed = Rc::new(Cell::new(false));
    let action_layer = layer.clone();
    let action_overlay = window_overlay.clone();
    let action_root = blurred_root.clone();
    let application = parent.application();
    action.connect_clicked(move |button| {
        if installed.get() {
            restart(application.as_ref());
            button.set_sensitive(false);
            return;
        }
        if started.replace(true) {
            dismiss_modal_layer(&action_layer, &action_overlay, action_root.as_ref());
            button.set_sensitive(false);
            return;
        }

        button.set_sensitive(false);
        cancel.set_sensitive(false);
        progress.set_visible(true);
        status.set_text("Starting download…");
        let receiver = services::install_update(download_url.clone());
        let progress = progress.clone();
        let status = status.clone();
        let action = button.clone();
        let installed = installed.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            loop {
                match receiver.try_recv() {
                    Ok(UpdateInstall::Downloading { downloaded, total }) => {
                        if let Some(total) = total.filter(|total| *total > 0) {
                            let fraction = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
                            progress.set_fraction(fraction);
                            status.set_text(&format!(
                                "Downloading… {:.0}%  ({:.1} of {:.1} MB)",
                                fraction * 100.0,
                                downloaded as f64 / 1_048_576.0,
                                total as f64 / 1_048_576.0,
                            ));
                        } else {
                            progress.pulse();
                            status.set_text(&format!(
                                "Downloading… {:.1} MB",
                                downloaded as f64 / 1_048_576.0
                            ));
                        }
                    }
                    Ok(UpdateInstall::Installing) => {
                        progress.set_fraction(1.0);
                        status.set_text("Download complete — verifying and installing…");
                    }
                    Ok(UpdateInstall::Installed) => {
                        progress.set_fraction(1.0);
                        status.set_text("Update installed — restart to apply");
                        action.set_label("Restart now");
                        action.add_css_class("suggested-action");
                        action.set_sensitive(true);
                        installed.set(true);
                        return glib::ControlFlow::Break;
                    }
                    Ok(UpdateInstall::Failed(message)) => {
                        status.set_text(&format!("Couldn’t install update: {message}"));
                        progress.add_css_class("error");
                        action.set_label("Close");
                        action.set_sensitive(true);
                        return glib::ControlFlow::Break;
                    }
                    Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(TryRecvError::Disconnected) => {
                        status.set_text("Couldn’t install update");
                        action.set_label("Close");
                        action.set_sensitive(true);
                        return glib::ControlFlow::Break;
                    }
                }
            }
        });
    });
}

fn shows_available_release_notes(result: &UpdateCheck) -> bool {
    matches!(result, UpdateCheck::Available { .. })
}

fn update_check_message(result: &UpdateCheck) -> String {
    match result {
        UpdateCheck::UpToDate => {
            format!("Up to date — version {}", env!("CARGO_PKG_VERSION"))
        }
        UpdateCheck::Available { release, .. } => format!(
            "Update available: <a href=\"{}\">v{}</a>",
            glib::markup_escape_text(&release.url),
            glib::markup_escape_text(&release.version),
        ),
        UpdateCheck::Failed(message) => format!(
            "Couldn't check for updates: {} · <a href=\"https://github.com/lgse/strata/releases/latest\">View releases on GitHub</a>",
            glib::markup_escape_text(message)
        ),
    }
}

fn keybindings_page() -> gtk::Widget {
    let content = page_content();
    append_heading(&content, "NAVIGATION");
    for (label, keys) in [
        ("Move through items", "J / K  or  ↑ / ↓"),
        ("Open folder", "L / → / Enter"),
        ("Go to parent", "H / ←"),
        ("Edit location", "Ctrl + L"),
        ("Filter items", "Ctrl + F"),
        ("Toggle sidebar", "Ctrl + B"),
    ] {
        append_keybinding(&content, label, keys);
    }

    append_heading(&content, "FILE OPERATIONS");
    for (label, keys) in [
        ("Create new folder", "Ctrl + Shift + N"),
        ("Cut", "Ctrl + X"),
        ("Copy", "Ctrl + C"),
        ("Paste", "Ctrl + V"),
    ] {
        append_keybinding(&content, label, keys);
    }

    append_heading(&content, "APPLICATION");
    for (label, keys) in [("Search", "Ctrl + K"), ("Open settings", "Ctrl + ,")] {
        append_keybinding(&content, label, keys);
    }

    scrollable_page(&content, Some("settings-keybindings-scroll"))
}

fn about_page() -> gtk::Widget {
    let content = page_content();
    content.add_css_class("about-page");

    let identity = gtk::Box::new(gtk::Orientation::Vertical, 7);
    identity.add_css_class("about-identity");
    identity.set_halign(gtk::Align::Center);

    let name = gtk::Label::new(Some("Strata"));
    name.add_css_class("about-name");
    let description = gtk::Label::new(Some(crate::build_info::DESCRIPTION));
    description.add_css_class("about-description");
    description.set_justify(gtk::Justification::Center);
    description.set_wrap(true);
    identity.append(&name);
    identity.append(&description);
    content.append(&identity);

    append_heading(&content, "BUILD INFORMATION");
    let build = gtk::Box::new(gtk::Orientation::Vertical, 0);
    build.add_css_class("about-details");
    append_about_detail(&build, "Version", crate::build_info::VERSION, false);
    append_about_detail(&build, "Commit", crate::build_info::COMMIT, true);
    content.append(&build);

    append_heading(&content, "PROJECT");
    let project = gtk::Box::new(gtk::Orientation::Vertical, 0);
    project.add_css_class("about-details");
    append_about_detail(&project, "Author", crate::build_info::AUTHOR, false);

    let repository = gtk::LinkButton::builder()
        .uri(crate::build_info::REPOSITORY)
        .tooltip_text("Open the Strata repository")
        .build();
    repository.add_css_class("about-repository");
    let repository_content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let repository_label = gtk::Label::new(Some("GitHub repository"));
    repository_label.set_xalign(0.0);
    repository_label.set_hexpand(true);
    repository_content.append(&repository_label);
    repository_content.append(&crate::assets::primary_icon(icons::EXTERNAL_LINK, 16));
    repository.set_child(Some(&repository_content));
    project.append(&repository);
    content.append(&project);

    scrollable_page(&content, None)
}

fn append_about_detail(container: &gtk::Box, label: &str, value: &str, monospace: bool) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("about-detail-row");
    let label = gtk::Label::new(Some(label));
    label.add_css_class("about-detail-label");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    let value = gtk::Label::new(Some(value));
    value.add_css_class("about-detail-value");
    value.set_selectable(true);
    if monospace {
        value.add_css_class("monospace");
    }
    row.append(&label);
    row.append(&value);
    container.append(&row);
}

fn append_keybinding(content: &gtk::Box, label: &str, keys: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("keybinding-row");
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    let keys = gtk::Label::new(Some(keys));
    keys.add_css_class("keybinding-keys");
    row.append(&label);
    row.append(&keys);
    content.append(&row);
}

fn theme_page(manager: Rc<ThemeManager>) -> (gtk::Widget, Vec<(gtk::FlowBox, u32)>) {
    let content = page_content();
    content.add_css_class("theme-page");

    let follow = gtk::Switch::builder()
        .active(manager.follows_omarchy())
        .valign(gtk::Align::Center)
        .build();
    if manager.is_omarchy_available() {
        append_heading(&content, "SYSTEM");
        let system = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        system.add_css_class("settings-option");
        let icon = crate::assets::primary_icon(icons::MONITOR, 22);
        icon.add_css_class("system-theme-icon");
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
        copy.set_hexpand(true);
        copy.set_valign(gtk::Align::Center);
        let system_title = gtk::Label::new(Some("Follow Omarchy"));
        system_title.set_xalign(0.0);
        system_title.add_css_class("settings-option-title");
        let system_description = gtk::Label::new(Some(
            "Use the active Omarchy Quattro theme and follow system theme changes.",
        ));
        system_description.set_xalign(0.0);
        system_description.set_wrap(true);
        system_description.add_css_class("settings-option-description");
        copy.append(&system_title);
        copy.append(&system_description);
        system.append(&icon);
        system.append(&copy);
        system.append(&follow);
        content.append(&system);
    }

    append_heading(&content, "THEMES");
    let packaged = gtk::FlowBox::builder()
        .column_spacing(12)
        .row_spacing(12)
        .max_children_per_line(3)
        .min_children_per_line(1)
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .build();
    packaged.add_css_class("theme-grid");
    content.append(&packaged);

    append_heading(&content, "YOUR THEMES");
    let custom = gtk::FlowBox::builder()
        .column_spacing(12)
        .row_spacing(12)
        .max_children_per_line(3)
        .min_children_per_line(1)
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .build();
    custom.add_css_class("theme-grid");
    content.append(&custom);

    let cards: ThemeCards = Rc::new(RefCell::new(Vec::new()));
    for theme in manager.themes() {
        let flow = if theme.custom { &custom } else { &packaged };
        append_theme_card(flow, theme, &manager, &follow, &cards);
    }

    let add = gtk::Button::new();
    add.add_css_class("add-theme-card");
    add.set_has_frame(false);
    let add_content = gtk::Box::new(gtk::Orientation::Vertical, 7);
    add_content.set_halign(gtk::Align::Center);
    add_content.set_valign(gtk::Align::Center);
    let plus = crate::assets::primary_icon(icons::PLUS, 22);
    let add_label = gtk::Label::new(Some("Add a theme"));
    add_content.append(&plus);
    add_content.append(&add_label);
    add.set_child(Some(&add_content));
    custom.insert(&add, -1);

    let (editor, editor_fields) = theme_editor(
        manager.clone(),
        custom.clone(),
        follow.clone(),
        cards.clone(),
    );
    editor.set_reveal_child(false);
    content.append(&editor);
    let shown_editor = editor.clone();
    add.connect_clicked(move |_| shown_editor.set_reveal_child(true));

    let scroller = scrollable_page(&content, None);
    let manager_for_follow = manager;
    follow.connect_active_notify(move |toggle| {
        let active = toggle.is_active();
        manager_for_follow.set_follow_omarchy(active);
        let selected_id = manager_for_follow.selected_id();
        for (id, card, check) in cards.borrow().iter() {
            let selected = !active && id == &selected_id;
            if selected {
                card.add_css_class("selected");
            } else {
                card.remove_css_class("selected");
            }
            check.set_visible(selected);
        }
    });
    (
        scroller,
        vec![(packaged, 3), (custom, 3), (editor_fields, 4)],
    )
}

fn append_theme_card(
    flow: &gtk::FlowBox,
    theme: Theme,
    manager: &Rc<ThemeManager>,
    follow: &gtk::Switch,
    cards: &ThemeCards,
) {
    let card = gtk::Button::new();
    card.add_css_class("theme-card");
    card.set_has_frame(false);
    card.set_overflow(gtk::Overflow::Visible);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let preview = gtk::Overlay::new();
    preview.set_child(Some(&theme_preview(&theme.tokens)));
    let check = gtk::Image::from_icon_name(icons::CHECK_ON_PRIMARY);
    check.add_css_class("theme-card-check");
    check.set_halign(gtk::Align::End);
    check.set_valign(gtk::Align::Start);
    check.set_margin_top(8);
    check.set_margin_end(8);
    check.set_pixel_size(10);
    preview.add_overlay(&check);
    content.append(&preview);
    let label_row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    let selected = !manager.follows_omarchy() && manager.selected_id() == theme.id;
    check.set_visible(selected);
    if selected {
        card.add_css_class("selected");
    }
    let label = gtk::Label::new(Some(&theme.tokens.name));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label_row.append(&label);
    content.append(&label_row);
    card.set_child(Some(&content));
    cards
        .borrow_mut()
        .push((theme.id.clone(), card.clone(), check));

    let theme_id = theme.id;
    let manager = manager.clone();
    let follow = follow.clone();
    let cards = cards.clone();
    card.connect_clicked(move |_| {
        manager.select_theme(&theme_id);
        follow.set_active(false);
        for (id, candidate, check) in cards.borrow().iter() {
            let selected = id == &theme_id;
            if selected {
                candidate.add_css_class("selected");
            } else {
                candidate.remove_css_class("selected");
            }
            check.set_visible(selected);
        }
    });
    flow.insert(&card, -1);
}

fn theme_preview(tokens: &ThemeTokens) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.add_css_class("theme-preview");
    area.set_content_width(190);
    area.set_content_height(72);
    let tokens = tokens.clone();
    area.set_draw_func(move |_, context, width, height| {
        let color = |value: &str| gdk::RGBA::parse(value).unwrap_or(gdk::RGBA::BLACK);
        let paint = |context: &gtk::cairo::Context, value: &str| {
            let value = color(value);
            context.set_source_rgba(
                f64::from(value.red()),
                f64::from(value.green()),
                f64::from(value.blue()),
                1.0,
            );
        };
        context.rounded_rectangle(0.0, 0.0, f64::from(width), f64::from(height), 6.0);
        context.clip();
        paint(context, &tokens.background);
        context.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
        let _ = context.fill();
        paint(context, &tokens.surface);
        context.rectangle(0.0, 0.0, f64::from(width) * 0.40, f64::from(height));
        let _ = context.fill();
        for (x, y, w, value) in [
            (10.0, 23.0, 45.0, &tokens.dim_text),
            (10.0, 36.0, 59.0, &tokens.accent),
            (10.0, 51.0, 39.0, &tokens.dim_text),
            (f64::from(width) * 0.45, 23.0, 47.0, &tokens.accent),
            (f64::from(width) * 0.45, 37.0, 83.0, &tokens.dim_text),
            (f64::from(width) * 0.45, 51.0, 66.0, &tokens.dim_text),
        ] {
            paint(context, value);
            context.rounded_rectangle(x, y, w, 5.0, 2.5);
            let _ = context.fill();
        }
    });
    area
}

fn theme_editor(
    manager: Rc<ThemeManager>,
    custom: gtk::FlowBox,
    follow: gtk::Switch,
    cards: ThemeCards,
) -> (gtk::Revealer, gtk::FlowBox) {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 12);
    panel.add_css_class("theme-editor");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Label::new(Some("Add a theme"));
    title.add_css_class("settings-option-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    header.append(&title);
    panel.append(&header);
    let name = gtk::Entry::builder().placeholder_text("Theme name").build();
    name.add_css_class("theme-name");
    panel.append(&name);

    let values = Rc::new(RefCell::new(manager.starter_tokens()));
    let fields = gtk::FlowBox::builder()
        .column_spacing(18)
        .row_spacing(10)
        .max_children_per_line(4)
        .min_children_per_line(1)
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .build();
    fields.add_css_class("theme-color-fields");
    for (label_text, field) in [
        ("Background", ColorField::Background),
        ("Surface", ColorField::Surface),
        ("Text", ColorField::Text),
        ("Accent", ColorField::Accent),
        ("Danger", ColorField::Danger),
        ("Muted", ColorField::Muted),
        ("Highlight", ColorField::Highlight),
        ("Border", ColorField::Border),
        ("Dim text", ColorField::DimText),
    ] {
        let field_row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        let label = gtk::Label::new(Some(label_text));
        label.set_xalign(0.0);
        let dialog = gtk::ColorDialog::builder()
            .title(format!("Choose {label_text}"))
            .with_alpha(false)
            .build();
        let picker = gtk::ColorDialogButton::new(Some(dialog));
        picker.add_css_class("theme-color-picker");
        if let Ok(color) = gdk::RGBA::parse(field.get(&values.borrow())) {
            picker.set_rgba(&color);
        }
        let values_for_color = values.clone();
        let manager_for_color = manager.clone();
        picker.connect_rgba_notify(move |picker| {
            field.set(
                &mut values_for_color.borrow_mut(),
                picker.rgba().to_string(),
            );
            manager_for_color.preview(&values_for_color.borrow());
        });
        field_row.append(&picker);
        field_row.append(&label);
        fields.insert(&field_row, -1);
    }
    panel.append(&fields);
    let error = gtk::Label::new(None);
    error.add_css_class("theme-editor-error");
    error.set_xalign(0.0);
    error.set_visible(false);
    panel.append(&error);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("theme-editor-cancel");
    let save = gtk::Button::with_label("Add theme");
    save.add_css_class("theme-editor-save");
    actions.append(&cancel);
    actions.append(&save);
    panel.append(&actions);
    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .child(&panel)
        .build();
    let hidden = revealer.clone();
    let manager_for_cancel = manager.clone();
    cancel.connect_clicked(move |_| {
        manager_for_cancel.cancel_preview();
        hidden.set_reveal_child(false);
    });
    let hidden = revealer.clone();
    save.connect_clicked(move |_| {
        let mut tokens = values.borrow().clone();
        tokens.name = name.text().trim().to_owned();
        match manager.save_custom_theme(tokens.clone()) {
            Ok(id) => {
                error.set_visible(false);
                append_theme_card(
                    &custom,
                    Theme {
                        id,
                        tokens,
                        custom: true,
                    },
                    &manager,
                    &follow,
                    &cards,
                );
                hidden.set_reveal_child(false);
            }
            Err(message) => {
                error.set_text(&message.to_string());
                error.set_visible(true);
            }
        }
    });
    (revealer, fields)
}

#[derive(Clone, Copy)]
enum ColorField {
    Background,
    Surface,
    Text,
    Accent,
    Danger,
    Muted,
    Highlight,
    Border,
    DimText,
}
impl ColorField {
    fn get(self, tokens: &ThemeTokens) -> &str {
        match self {
            Self::Background => &tokens.background,
            Self::Surface => &tokens.surface,
            Self::Text => &tokens.text,
            Self::Accent => &tokens.accent,
            Self::Danger => &tokens.danger,
            Self::Muted => &tokens.muted,
            Self::Highlight => &tokens.highlight,
            Self::Border => &tokens.border,
            Self::DimText => &tokens.dim_text,
        }
    }
    fn set(self, tokens: &mut ThemeTokens, value: String) {
        *match self {
            Self::Background => &mut tokens.background,
            Self::Surface => &mut tokens.surface,
            Self::Text => &mut tokens.text,
            Self::Accent => &mut tokens.accent,
            Self::Danger => &mut tokens.danger,
            Self::Muted => &mut tokens.muted,
            Self::Highlight => &mut tokens.highlight,
            Self::Border => &mut tokens.border,
            Self::DimText => &mut tokens.dim_text,
        } = value;
    }
}

fn navigation_button(
    icon: &str,
    label: &str,
    active: bool,
) -> (gtk::Button, gtk::Label, gtk::Box, gtk::Image, gtk::Image) {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let primary_icon = crate::assets::primary_icon(icon, 18);
    primary_icon.set_visible(active);
    let text_icon = crate::assets::text_icon(icon, 18);
    text_icon.set_visible(!active);
    let text = gtk::Label::new(Some(label));
    text.set_xalign(0.0);
    content.append(&primary_icon);
    content.append(&text_icon);
    content.append(&text);
    let button = gtk::Button::builder()
        .child(&content)
        .tooltip_text(label)
        .build();
    button.set_has_frame(false);
    (button, text, content, primary_icon, text_icon)
}

fn scrollable_page(content: &gtk::Box, class: Option<&str>) -> gtk::Widget {
    content.set_hexpand(true);
    let scroller = gtk::ScrolledWindow::builder()
        .child(content)
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

fn page_content() -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.add_css_class("settings-preferences");
    content
}

fn settings_option(title: &str, description: &str, active: bool) -> (gtk::Box, gtk::Switch) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("settings-option");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(title));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let description = gtk::Label::new(Some(description));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.add_css_class("settings-option-description");
    copy.append(&title);
    copy.append(&description);
    let toggle = gtk::Switch::builder()
        .active(active)
        .valign(gtk::Align::Center)
        .build();
    row.append(&copy);
    row.append(&toggle);
    (row, toggle)
}

fn append_heading(container: &gtk::Box, text: &str) -> gtk::Label {
    let heading = gtk::Label::new(Some(text));
    heading.set_xalign(0.0);
    heading.add_css_class("menu-heading");
    container.append(&heading);
    heading
}

trait RoundedRectangle {
    fn rounded_rectangle(&self, x: f64, y: f64, width: f64, height: f64, radius: f64);
}
impl RoundedRectangle for gtk::cairo::Context {
    fn rounded_rectangle(&self, x: f64, y: f64, width: f64, height: f64, radius: f64) {
        let degrees = std::f64::consts::PI / 180.0;
        self.new_sub_path();
        self.arc(x + width - radius, y + radius, radius, -90.0 * degrees, 0.0);
        self.arc(
            x + width - radius,
            y + height - radius,
            radius,
            0.0,
            90.0 * degrees,
        );
        self.arc(
            x + radius,
            y + height - radius,
            radius,
            90.0 * degrees,
            180.0 * degrees,
        );
        self.arc(
            x + radius,
            y + radius,
            radius,
            180.0 * degrees,
            270.0 * degrees,
        );
        self.close_path();
    }
}
