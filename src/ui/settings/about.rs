// SPDX-License-Identifier: GPL-3.0-or-later

use gtk::prelude::*;

use crate::{assets::icons, services};

use super::{append_heading, page_content, scrollable_page};

pub(super) fn about_page() -> gtk::Widget {
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
    let version = crate::build_info::installed_version().to_string();
    append_about_detail(&build, "Version", &version, false);
    let build_kind = crate::build_info::build_kind();
    if build_kind != services::BuildKind::Stable {
        append_about_detail(&build, "Build", build_kind.label(), false);
    }
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
