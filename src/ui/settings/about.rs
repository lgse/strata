// SPDX-License-Identifier: MIT

use super::{page_content, scrollable_page, settings_group};
use crate::{assets::icons, services};
use gtk::prelude::*;

pub(super) fn about_page() -> gtk::Widget {
    let content = page_content();
    content.add_css_class("about-page");
    let identity = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    identity.add_css_class("about-identity");
    super::search::tag(&identity, "Version information");
    let icon = gtk::Image::from_resource("/io/github/lgse/Strata/brand/strata-logo-white.svg");
    icon.set_pixel_size(36);
    let logo = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    logo.add_css_class("about-logo");
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    icon.set_hexpand(true);
    logo.append(&icon);
    logo.set_valign(gtk::Align::Center);
    logo.set_halign(gtk::Align::Start);
    logo.set_hexpand(false);
    identity.append(&logo);
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 6);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let name = gtk::Label::new(Some("Strata"));
    name.add_css_class("about-name");
    heading.append(&name);
    let kind = crate::build_info::build_kind();
    if kind != services::BuildKind::Stable {
        let badge = gtk::Label::new(Some(kind.label()));
        badge.add_css_class("prerelease-badge");
        badge.set_valign(gtk::Align::Center);
        heading.append(&badge);
    }
    copy.append(&heading);
    let description = gtk::Label::new(Some("A keyboard-first file manager for Linux."));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.add_css_class("about-description");
    copy.append(&description);
    identity.append(&copy);
    let button = gtk::Button::with_label("Copy version info");
    button.add_css_class("settings-update-check");
    button.set_valign(gtk::Align::Center);
    button.connect_clicked(|button| button.clipboard().set_text(&version_info()));
    identity.append(&button);
    content.append(&identity);

    let build = settings_group(&content, "BUILD");
    super::search::tag(&build, "Version information");
    append_about_detail(
        &build,
        "Version",
        &crate::build_info::installed_version().to_string(),
    );
    append_about_detail(&build, "Commit", crate::build_info::COMMIT);
    append_about_detail(&build, "Toolkit", &toolkit_version());
    let links = settings_group(&content, "LINKS");
    for (label, icon, uri, detail) in [
        (
            "Website",
            icons::GLOBE,
            "https://stratafiles.io/".to_owned(),
            "",
        ),
        (
            "Source code",
            icons::CODE_XML,
            crate::build_info::REPOSITORY.to_owned(),
            "",
        ),
        (
            "Report an issue",
            icons::BUG,
            format!("{}/issues/new/choose", crate::build_info::REPOSITORY),
            "",
        ),
        (
            "License",
            icons::SCALE,
            format!("{}/blob/main/LICENSE", crate::build_info::REPOSITORY),
            "MIT",
        ),
    ] {
        let button = gtk::LinkButton::builder().uri(&uri).build();
        button.add_css_class("about-repository");
        super::search::tag(&button, label);
        crate::ui::accessibility::set_label(&button, label);
        let row = super::wrap::WrapRow::new(16);
        row.append(&crate::assets::primary_icon(icon, 18));
        let title = gtk::Label::new(Some(label));
        title.set_xalign(0.0);
        title.set_hexpand(true);
        title.set_wrap(true);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        row.append(&title);
        let metadata = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        let detail = gtk::Label::new(Some(detail));
        detail.add_css_class("settings-option-description");
        detail.add_css_class("settings-control-label");
        detail.set_visible(!detail.text().is_empty());
        metadata.append(&detail);
        metadata.append(&crate::assets::primary_icon(icons::EXTERNAL_LINK, 16));
        row.append(&metadata);
        button.set_child(Some(&row));
        links.append(&button);
    }
    scrollable_page(&content, None)
}

fn toolkit_version() -> String {
    format!(
        "GTK {}.{}.{}",
        gtk::major_version(),
        gtk::minor_version(),
        gtk::micro_version()
    )
}

fn version_info() -> String {
    format!(
        "Strata {}\n{}\nCommit: {}\nToolkit: {}\nAuthor: {}\nLicense: MIT",
        crate::build_info::installed_version(),
        crate::build_info::DESCRIPTION,
        crate::build_info::COMMIT,
        toolkit_version(),
        crate::build_info::AUTHOR
    )
}

fn append_about_detail(container: &gtk::Box, label: &str, value: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("about-detail-row");
    let label = gtk::Label::new(Some(label));
    label.add_css_class("about-detail-label");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    let value = gtk::Label::new(Some(value));
    value.add_css_class("about-detail-value");
    value.add_css_class("settings-nowrap");
    value.set_selectable(true);
    value.set_xalign(1.0);
    row.append(&label);
    row.append(&value);
    container.append(&row);
}
