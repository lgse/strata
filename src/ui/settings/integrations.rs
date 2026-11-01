// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

use gtk::{gdk, gio, glib, prelude::*};

use super::{
    install_command::install_command, page_content, scrollable_page, search, settings_group,
};
use crate::{
    assets::{self, icons},
    services::{
        integration_catalog::{self, Category, Entry, Installation, RequirementResult, Status},
        package_manager::PackageManager,
    },
    ui::accessibility,
};

#[cfg(test)]
mod tests;

const TITLE: &str = "Integrations";
const CARD_CHARS: i32 = 28;
const MAX_COLUMNS: u32 = 3;

type Checker = Arc<dyn Fn(&Entry) -> Vec<RequirementResult> + Send + Sync>;

/// Where the page reads the catalog and the registry from; tests substitute both.
pub(super) struct Source {
    pub catalog: Box<dyn Fn() -> Vec<Entry>>,
    pub root: Option<PathBuf>,
    pub check: Checker,
    /// Shows the registration directory in the browser, when a window hosts the page.
    pub open_folder: Option<super::OpenFolder>,
    pub reload: Rc<dyn Fn()>,
}

impl Default for Source {
    fn default() -> Self {
        Self {
            catalog: Box::new(integration_catalog::catalog),
            root: crate::services::file_providers::config_root(),
            check: Arc::new(integration_catalog::check_requirements),
            open_folder: None,
            reload: Rc::new(crate::ui::file_providers::reload),
        }
    }
}

pub(super) fn integrations_page(open_folder: Option<super::OpenFolder>) -> gtk::Widget {
    integrations_page_with(Source {
        open_folder,
        ..Source::default()
    })
}

pub(super) fn integrations_page_with(source: Source) -> gtk::Widget {
    let content = page_content();
    content.add_css_class("settings-integrations-page");
    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row.add_css_class("settings-option");
    search::tag(&row, TITLE);
    // One child keeps the copy stacked when the responsive layout widens rows.
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    let title = gtk::Label::new(Some(&crate::i18n::tr(TITLE)));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let description = gtk::Label::new(Some(&crate::i18n::tr(
        "Optional integrations bundled with Strata connect the context menu to other apps and services, such as sending files to a phone. They run with your permissions, like any program you install.",
    )));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.add_css_class("settings-option-description");
    copy.append(&title);
    copy.append(&description);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.add_css_class("settings-integration-actions");
    actions.set_halign(gtk::Align::End);
    actions.set_margin_top(6);
    copy.append(&actions);
    row.append(&copy);
    settings_group(&content, "").append(&row);

    let installed = gtk::Box::new(gtk::Orientation::Vertical, 0);
    search::tag(&installed, TITLE);
    content.append(&installed);

    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    toolbar.add_css_class("settings-library-toolbar");
    search::tag(&toolbar, TITLE);
    let (heading, count) = counted_heading("CATALOG");
    heading.set_hexpand(true);
    toolbar.append(&heading);
    content.append(&toolbar);
    let filters = super::wrap::WrapRow::new(6);
    filters.add_css_class("integration-filters");
    search::tag(&filters, TITLE);
    content.append(&filters);
    let grid = gtk::FlowBox::builder()
        .column_spacing(12)
        .row_spacing(12)
        .min_children_per_line(1)
        .max_children_per_line(MAX_COLUMNS)
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .build();
    grid.add_css_class("integration-grid");
    search::tag(&grid, TITLE);
    content.append(&grid);
    let empty = gtk::Label::new(Some(&crate::i18n::tr(
        "No integrations are bundled with this build.",
    )));
    empty.set_xalign(0.0);
    empty.set_wrap(true);
    empty.add_css_class("settings-section-description");
    empty.add_css_class("integration-empty");
    empty.set_visible(false);
    content.append(&empty);

    let filter = Rc::new(Filter::default());
    let filtering = filter.clone();
    grid.set_filter_func(move |child| {
        child
            .child()
            .is_some_and(|card| filtering.matches(&card.widget_name()))
    });

    let page = Rc::new(Page {
        source,
        installed,
        count,
        filters,
        grid,
        empty,
        filter,
        expanded: RefCell::default(),
        requirements: RefCell::default(),
        manager: Cell::new(None),
        generation: Cell::new(0),
    });
    let refresh = icon_label_button(icons::REFRESH, "Refresh");
    refresh.set_widget_name("integrations-refresh");
    let refreshing = Rc::downgrade(&page);
    refresh.connect_clicked(move |_| {
        if let Some(page) = refreshing.upgrade() {
            (page.source.reload)();
            page.requirements.borrow_mut().clear();
            page.rebuild();
        }
    });
    actions.append(&refresh);
    if let (Some(root), Some(open_folder)) = (&page.source.root, page.source.open_folder.clone()) {
        let open = icon_label_button(icons::FOLDER_OPEN, "Open folder");
        open.set_widget_name("integrations-open-folder");
        let root = root.clone();
        open.connect_clicked(move |_| open_folder(root.clone()));
        actions.append(&open);
    }
    // The registry can change outside this page, so every visit rescans it.
    content.connect_map(move |_| page.rebuild());
    scrollable_page(&content, None)
}

/// The catalog's category filter; cards are keyed by widget name.
#[derive(Default)]
struct Filter {
    category: Cell<Option<Category>>,
    cards: RefCell<HashMap<String, Category>>,
}

impl Filter {
    fn matches(&self, card: &str) -> bool {
        let category = self.cards.borrow().get(card).copied();
        self.category
            .get()
            .is_none_or(|selected| category.is_none_or(|category| category == selected))
    }
}

struct Page {
    source: Source,
    installed: gtk::Box,
    count: gtk::Label,
    filters: super::wrap::WrapRow,
    grid: gtk::FlowBox,
    empty: gtk::Label,
    filter: Rc<Filter>,
    /// Entries whose requirements are shown, kept across rebuilds.
    expanded: RefCell<HashSet<String>>,
    /// Requirement results by entry id, filled by a worker after the first build.
    requirements: RefCell<HashMap<String, Vec<RequirementResult>>>,
    manager: Cell<Option<PackageManager>>,
    generation: Cell<u64>,
}

struct Row {
    entry: Entry,
    status: Status,
    enabled: bool,
}

impl Page {
    fn rebuild(self: &Rc<Self>) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let catalog = (self.source.catalog)();
        let root = self.source.root.clone();
        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let scanned = gio::spawn_blocking(move || {
                let rows: Vec<Row> = catalog
                    .into_iter()
                    .map(|entry| Row {
                        status: root.as_deref().map_or(Status::NotInstalled, |root| {
                            integration_catalog::status(&entry, root)
                        }),
                        enabled: root
                            .as_deref()
                            .is_none_or(|root| integration_catalog::enabled(root, &entry.id)),
                        entry,
                    })
                    .collect();
                let manual = root.as_deref().map_or_else(Vec::new, |root| {
                    let catalog: Vec<Entry> = rows.iter().map(|row| row.entry.clone()).collect();
                    integration_catalog::manual_installations(root, &catalog)
                });
                (rows, manual, PackageManager::detect())
            })
            .await;
            let Some(page) = page.upgrade() else {
                return;
            };
            if page.generation.get() != generation {
                return;
            }
            if let Ok((rows, manual, manager)) = scanned {
                page.manager.set(manager);
                page.show(rows, manual);
            }
        });
    }

    fn show(self: &Rc<Self>, rows: Vec<Row>, manual: Vec<Installation>) {
        self.show_installed(&rows, manual);
        self.show_catalog(&rows);
        let unchecked: Vec<Entry> = rows
            .iter()
            .filter(|row| !self.requirements.borrow().contains_key(&row.entry.id))
            .map(|row| row.entry.clone())
            .collect();
        if !unchecked.is_empty() {
            self.check(unchecked);
        }
    }

    fn show_installed(self: &Rc<Self>, rows: &[Row], manual: Vec<Installation>) {
        while let Some(child) = self.installed.first_child() {
            self.installed.remove(&child);
        }
        let mut installed: Vec<(String, gtk::Box)> = rows
            .iter()
            .filter(|row| row.status != Status::NotInstalled)
            .map(|row| {
                let name = crate::i18n::tr(&row.entry.name);
                (name.to_lowercase(), self.catalog_row(row, &name))
            })
            .collect();
        installed.extend(manual.into_iter().map(|installation| {
            (
                installation.name.to_lowercase(),
                self.manual_row(installation),
            )
        }));
        if installed.is_empty() {
            return;
        }
        installed.sort_by(|a, b| a.0.cmp(&b.0));
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        toolbar.add_css_class("settings-library-toolbar");
        let (heading, count) = counted_heading("INSTALLED");
        count.set_text(&crate::i18n::integer(installed.len() as u64));
        toolbar.append(&heading);
        self.installed.append(&toolbar);
        let group = gtk::Box::new(gtk::Orientation::Vertical, 0);
        group.add_css_class("settings-group");
        group.set_overflow(gtk::Overflow::Hidden);
        for (_, row) in installed {
            group.append(&row);
        }
        self.installed.append(&group);
    }

    fn show_catalog(self: &Rc<Self>, rows: &[Row]) {
        self.grid.remove_all();
        self.count
            .set_text(&crate::i18n::integer(rows.len() as u64));
        let heights = gtk::SizeGroup::new(gtk::SizeGroupMode::Vertical);
        let mut cards = HashMap::new();
        for row in rows {
            let name = crate::i18n::tr(&row.entry.name);
            let card = self.catalog_card(row, &name, &heights);
            cards.insert(card.widget_name().to_string(), row.entry.category);
            self.grid.append(&card);
        }
        let mut child = self.grid.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            widget.set_focusable(false);
            widget.set_valign(gtk::Align::Start);
        }
        self.filter.cards.replace(cards);
        if self
            .filter
            .category
            .get()
            .is_some_and(|selected| rows.iter().all(|row| row.entry.category != selected))
        {
            self.filter.category.set(None);
        }
        self.show_filters(rows);
        self.apply_filter();
        self.empty.set_visible(rows.is_empty());
    }

    fn show_filters(self: &Rc<Self>, rows: &[Row]) {
        while let Some(child) = self.filters.first_child() {
            child.unparent();
        }
        let mut options = vec![(None, rows.len())];
        for category in Category::ALL {
            let count = rows
                .iter()
                .filter(|row| row.entry.category == category)
                .count();
            if count > 0 {
                options.push((Some(category), count));
            }
        }
        self.filters.set_visible(options.len() > 2);
        let mut group = None::<gtk::ToggleButton>;
        for (category, count) in options {
            let label = crate::i18n::tr(category.map_or("All", category_label));
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            content.append(&gtk::Label::new(Some(&label)));
            let number = gtk::Label::new(Some(&crate::i18n::integer(count as u64)));
            number.add_css_class("integration-filter-count");
            content.append(&number);
            let chip = gtk::ToggleButton::builder().child(&content).build();
            chip.add_css_class("action-library-category");
            accessibility::set_label(
                &chip,
                &rust_i18n::t!(
                    "%{label}: %{value}",
                    label = label,
                    value = crate::i18n::integer(count as u64)
                ),
            );
            chip.set_group(group.as_ref());
            group.get_or_insert_with(|| chip.clone());
            chip.set_active(self.filter.category.get() == category);
            let page = Rc::downgrade(self);
            chip.connect_toggled(move |chip| {
                if let Some(page) = page.upgrade()
                    && chip.is_active()
                {
                    page.filter.category.set(category);
                    page.apply_filter();
                }
            });
            self.filters.append(&chip);
        }
    }

    fn apply_filter(&self) {
        let shown = self
            .filter
            .cards
            .borrow()
            .keys()
            .filter(|card| self.filter.matches(card))
            .count();
        // FlowBox keeps space for empty columns, so cap them at the cards shown.
        self.grid
            .set_max_children_per_line((shown as u32).clamp(1, MAX_COLUMNS));
        self.grid.invalidate_filter();
    }

    fn check(self: &Rc<Self>, entries: Vec<Entry>) {
        let page = Rc::downgrade(self);
        let generation = self.generation.get();
        let check = self.source.check.clone();
        glib::spawn_future_local(async move {
            let results = gio::spawn_blocking(move || {
                entries
                    .into_iter()
                    .map(|entry| {
                        let results = check(&entry);
                        (entry.id, results)
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            let Some(page) = page.upgrade() else {
                return;
            };
            if let Ok(results) = results {
                page.requirements.borrow_mut().extend(results);
                if page.generation.get() == generation {
                    page.rebuild();
                }
            }
        });
    }

    fn catalog_card(self: &Rc<Self>, row: &Row, name: &str, heights: &gtk::SizeGroup) -> gtk::Box {
        let entry = &row.entry;
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("integration-card");
        card.set_widget_name(&format!("integration-{}-card", entry.id));
        card.set_size_request(240, -1);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        body.add_css_class("integration-card-body");
        heights.add_widget(&body);
        body.append(&logo_tile(entry.icon_bytes(), entry.category));
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 6);
        copy.set_hexpand(true);
        let title = gtk::Label::new(Some(name));
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.set_max_width_chars(CARD_CHARS);
        title.add_css_class("settings-option-title");
        copy.append(&title);
        let description = note(&crate::i18n::tr(&entry.description));
        // Wrapping labels otherwise report their unwrapped width, which
        // leaves one card per grid line.
        description.set_max_width_chars(CARD_CHARS);
        description.set_lines(2);
        description.set_ellipsize(gtk::pango::EllipsizeMode::End);
        copy.append(&description);
        let message = note("");
        message.add_css_class("integration-message");
        message.set_visible(false);
        copy.append(&message);
        body.append(&copy);
        let actions = gtk::Box::new(gtk::Orientation::Vertical, 6);
        actions.set_valign(gtk::Align::Start);
        body.append(&actions);
        card.append(&body);
        if row.status == Status::NotInstalled {
            let context = Rc::new(RowContext {
                page: Rc::downgrade(self),
                id: entry.id.clone(),
                name: name.to_owned(),
                message,
                confirmation: gtk::Box::new(gtk::Orientation::Vertical, 0),
                actions: actions.clone(),
            });
            let install = icon_label_button(icons::DOWNLOADS, "Install");
            accessibility::set_label(&install, &rust_i18n::t!("Install %{name}", name = name));
            install.set_widget_name(&format!("integration-{}-install", entry.id));
            actions.append(&install);
            let entry = entry.clone();
            install.connect_clicked(move |_| context.install(entry.clone()));
        } else {
            let installed = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            installed.add_css_class("integration-installed");
            installed.append(&assets::primary_icon(icons::CHECK, 14));
            installed.append(&gtk::Label::new(Some(&crate::i18n::tr("Installed"))));
            actions.append(&installed);
        }
        card.append(&self.requirements_section(entry, name));
        card
    }

    fn requirements_section(self: &Rc<Self>, entry: &Entry, name: &str) -> gtk::Box {
        let section = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let results = self.requirements.borrow().get(&entry.id).cloned();
        let met = results
            .as_ref()
            .map(|results| results.iter().all(|result| result.satisfied));
        let expanded = self.expanded.borrow().contains(&entry.id);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let title = gtk::Label::new(None);
        title.add_css_class("settings-option-description");
        heading.append(&title);
        if let Some(met) = met {
            let status = if met {
                assets::primary_icon(icons::CIRCLE_CHECK, 14)
            } else {
                assets::danger_icon(icons::CIRCLE_X, 14)
            };
            status.add_css_class(if met { "met" } else { "missing" });
            heading.append(&status);
        }
        let chevron = assets::primary_icon(icons::CHEVRON_RIGHT, 14);
        chevron.add_css_class("integration-chevron");
        chevron.set_hexpand(true);
        chevron.set_halign(gtk::Align::End);
        heading.append(&chevron);
        let toggle = gtk::Button::builder().child(&heading).build();
        toggle.add_css_class("integration-requirements-toggle");
        toggle.set_widget_name(&format!("integration-{}-requirements", entry.id));
        accessibility::set_label(
            &toggle,
            &rust_i18n::t!("Requirements for %{name}", name = name),
        );
        if let Some(met) = met {
            accessibility::set_description(
                &toggle,
                Some(&crate::i18n::tr(if met {
                    "All requirements are met."
                } else {
                    "Some requirements are missing."
                })),
            );
        }
        let details = gtk::Revealer::builder()
            .child(&self.requirements_details(results.as_deref()))
            .transition_duration(0)
            .reveal_child(expanded)
            .build();
        let show = {
            let toggle = toggle.clone();
            let title = title.clone();
            move |expanded: bool| {
                title.set_text(&crate::i18n::tr(if expanded {
                    "Hide requirements"
                } else {
                    "Requirements"
                }));
                toggle.update_state(&[gtk::accessible::State::Expanded(Some(expanded))]);
                if expanded {
                    toggle.add_css_class("expanded");
                } else {
                    toggle.remove_css_class("expanded");
                }
            }
        };
        show(expanded);
        let page = Rc::downgrade(self);
        let id = entry.id.clone();
        let revealer = details.clone();
        toggle.connect_clicked(move |_| {
            let expanded = !revealer.reveals_child();
            revealer.set_reveal_child(expanded);
            show(expanded);
            if let Some(page) = page.upgrade() {
                let mut ids = page.expanded.borrow_mut();
                if expanded {
                    ids.insert(id.clone());
                } else {
                    ids.remove(&id);
                }
            }
        });
        section.append(&toggle);
        section.append(&details);
        section
    }

    fn requirements_details(&self, results: Option<&[RequirementResult]>) -> gtk::Box {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 10);
        container.add_css_class("integration-requirements");
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chips.add_css_class("integration-requirement-list");
        let label = gtk::Label::new(Some(&crate::i18n::tr("Requires:")));
        label.add_css_class("settings-option-description");
        chips.append(&label);
        container.append(&chips);
        let Some(results) = results else {
            let checking = gtk::Label::new(Some(&crate::i18n::tr("Checking…")));
            checking.add_css_class("settings-option-description");
            chips.append(&checking);
            return container;
        };
        let mut hints = Vec::new();
        for result in results {
            let chip = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            chip.add_css_class("integration-requirement");
            let icon = if result.satisfied {
                assets::primary_icon(icons::CIRCLE_CHECK, 14)
            } else {
                assets::danger_icon(icons::CIRCLE_X, 14)
            };
            chip.append(&icon);
            let name = result.requirement.label();
            let text = gtk::Label::new(Some(name));
            text.add_css_class("settings-option-description");
            chip.append(&text);
            chip.add_css_class(if result.satisfied {
                "available"
            } else {
                "missing"
            });
            accessibility::set_label(
                &text,
                &rust_i18n::t!(
                    "%{label}: %{value}",
                    label = name,
                    value = crate::i18n::tr(if result.satisfied {
                        "Available"
                    } else {
                        "Missing"
                    })
                ),
            );
            chips.append(&chip);
            if !result.satisfied
                && let Some(command) = result.requirement.install_command(self.manager.get())
                && !hints.contains(&command)
            {
                hints.push(command);
            }
        }
        for command in hints {
            let commands = gtk::Box::new(gtk::Orientation::Vertical, 6);
            commands.append(&note(&crate::i18n::tr(
                "Install missing dependencies with:",
            )));
            commands.append(&install_command(
                &command,
                &crate::i18n::tr(
                    "The command opened in your terminal. Select Refresh when it finishes.",
                ),
            ));
            container.append(&commands);
        }
        container
    }

    fn catalog_row(self: &Rc<Self>, row: &Row, name: &str) -> gtk::Box {
        let entry = &row.entry;
        let (update, modified) = match row.status {
            Status::Installed { update, modified } => (update, modified),
            Status::Manual | Status::NotInstalled => (false, false),
        };
        let (widget, context) = self.installed_row(
            &entry.id,
            name,
            logo_tile(entry.icon_bytes(), entry.category),
            if row.status == Status::Manual {
                "Manually installed"
            } else {
                "Installed via catalog"
            },
        );
        context.toggle(row.enabled);
        if update {
            let button = context.button(
                "Update",
                icons::REFRESH,
                &rust_i18n::t!("Update %{name}", name = name),
                false,
            );
            let entry = entry.clone();
            let updating = context.clone();
            button.connect_clicked(move |_| {
                if modified {
                    let entry = entry.clone();
                    let context = updating.clone();
                    updating.confirm(
                        &crate::i18n::tr("The installed copy has local changes. Updating replaces them with the bundled files."),
                        "Replace",
                        rust_i18n::t!("Confirm replacing %{name}", name = updating.name).into_owned(),
                        Box::new(move || context.install(entry.clone())),
                    );
                } else {
                    updating.install(entry.clone());
                }
            });
        }
        let manual = row.status == Status::Manual;
        let question = if manual {
            Some("This copy was not installed from the catalog. Its folder is deleted.")
        } else if modified {
            Some("The installed copy has local changes that are deleted with it.")
        } else {
            None
        };
        context.remove_button(question);
        widget
    }

    fn manual_row(self: &Rc<Self>, installation: Installation) -> gtk::Box {
        let (widget, context) = self.installed_row(
            &installation.id,
            &installation.name,
            logo_tile(installation.icon.as_deref(), Category::Other),
            "Manually installed",
        );
        context.toggle(installation.enabled);
        context.remove_button(Some(
            "This copy was not installed from the catalog. Its folder is deleted.",
        ));
        widget
    }

    fn installed_row(
        self: &Rc<Self>,
        id: &str,
        name: &str,
        logo: gtk::Widget,
        origin: &str,
    ) -> (gtk::Box, Rc<RowContext>) {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 6);
        widget.add_css_class("settings-option");
        widget.add_css_class("integration-row");
        widget.set_widget_name(&format!("integration-{id}"));
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        heading.append(&logo);
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
        copy.set_hexpand(true);
        copy.set_valign(gtk::Align::Center);
        let title = gtk::Label::new(Some(name));
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.add_css_class("settings-option-title");
        copy.append(&title);
        let origin = gtk::Label::new(Some(&crate::i18n::tr(origin)));
        origin.set_xalign(0.0);
        origin.add_css_class("settings-option-description");
        origin.add_css_class("integration-origin");
        copy.append(&origin);
        heading.append(&copy);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.add_css_class("integration-actions");
        actions.set_valign(gtk::Align::Center);
        heading.append(&actions);
        widget.append(&heading);
        let message = note("");
        message.add_css_class("integration-message");
        message.set_visible(false);
        widget.append(&message);
        let confirmation = gtk::Box::new(gtk::Orientation::Vertical, 8);
        confirmation.add_css_class("integration-confirmation");
        confirmation.set_visible(false);
        widget.append(&confirmation);
        let context = Rc::new(RowContext {
            page: Rc::downgrade(self),
            id: id.to_owned(),
            name: name.to_owned(),
            message,
            confirmation,
            actions,
        });
        (widget, context)
    }
}

struct RowContext {
    page: std::rc::Weak<Page>,
    id: String,
    name: String,
    message: gtk::Label,
    confirmation: gtk::Box,
    actions: gtk::Box,
}

impl RowContext {
    /// A borderless icon button in the row heading.
    fn button(&self, label: &str, icon: &str, accessible: &str, danger: bool) -> gtk::Button {
        let image = if danger {
            assets::danger_icon(icon, assets::CHROME_ICON_PX)
        } else {
            assets::text_icon(icon, assets::CHROME_ICON_PX)
        };
        image.set_halign(gtk::Align::Center);
        image.set_valign(gtk::Align::Center);
        let button = gtk::Button::builder()
            .child(&image)
            .tooltip_text(crate::i18n::tr(label))
            .build();
        accessibility::set_label(&button, accessible);
        button.add_css_class("settings-action-button");
        button.add_css_class("settings-action-icon-button");
        button.add_css_class("borderless");
        if danger {
            button.add_css_class("danger");
        }
        button.set_widget_name(&format!("integration-{}-{}", self.id, label.to_lowercase()));
        self.actions.append(&button);
        button
    }

    /// A labelled switch that keeps the registration installed but stops it
    /// loading, separated from the row's buttons.
    fn toggle(self: &Rc<Self>, enabled: bool) {
        let group = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        group.add_css_class("integration-toggle");
        group.set_valign(gtk::Align::Center);
        let state = gtk::Label::new(Some(&toggle_label(enabled)));
        state.add_css_class("settings-option-description");
        group.append(&state);
        let switch = gtk::Switch::builder()
            .active(enabled)
            .valign(gtk::Align::Center)
            .build();
        accessibility::set_label(&switch, &rust_i18n::t!("Enable %{name}", name = self.name));
        switch.set_widget_name(&format!("integration-{}-enabled", self.id));
        let context = self.clone();
        switch.connect_state_set(move |_, enabled| {
            state.set_text(&toggle_label(enabled));
            context.set_enabled(enabled);
            glib::Propagation::Proceed
        });
        group.append(&switch);
        self.actions.append(&group);
        let separator = gtk::Separator::new(gtk::Orientation::Vertical);
        separator.add_css_class("integration-separator");
        self.actions.append(&separator);
    }

    /// Removes the registration, first asking `question` when there is one.
    fn remove_button(self: &Rc<Self>, question: Option<&'static str>) {
        let remove = self.button(
            "Remove",
            icons::TRASH,
            &rust_i18n::t!("Remove %{name}", name = self.name),
            true,
        );
        let context = self.clone();
        remove.connect_clicked(move |_| match question {
            Some(question) => {
                let removing = context.clone();
                context.confirm(
                    &crate::i18n::tr(question),
                    "Remove",
                    rust_i18n::t!("Confirm removing %{name}", name = context.name).into_owned(),
                    Box::new(move || removing.remove()),
                );
            }
            None => context.remove(),
        });
    }

    fn busy(&self, busy: bool) {
        let mut child = self.actions.first_child();
        while let Some(button) = child {
            child = button.next_sibling();
            button.set_sensitive(!busy);
        }
    }

    fn confirm(
        self: &Rc<Self>,
        question: &str,
        accept: &str,
        accessible: String,
        run: Box<dyn Fn()>,
    ) {
        while let Some(child) = self.confirmation.first_child() {
            self.confirmation.remove(&child);
        }
        let text = note(question);
        self.confirmation.append(&text);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.add_css_class("settings-integration-actions");
        buttons.set_halign(gtk::Align::End);
        let cancel = gtk::Button::with_label(&crate::i18n::tr("Cancel"));
        cancel.add_css_class("action-dialog-cancel");
        let proceed = named_button(accept, &accessible);
        proceed.add_css_class("action-dialog-confirm");
        proceed.set_widget_name(&format!("integration-{}-confirm", self.id));
        buttons.append(&cancel);
        buttons.append(&proceed);
        self.confirmation.append(&buttons);
        self.confirmation.set_visible(true);
        self.busy(true);
        let context = self.clone();
        cancel.connect_clicked(move |_| {
            context.confirmation.set_visible(false);
            context.busy(false);
        });
        let context = self.clone();
        proceed.connect_clicked(move |_| {
            context.confirmation.set_visible(false);
            run();
        });
    }

    fn install(self: &Rc<Self>, entry: Entry) {
        let Some(page) = self.page.upgrade() else {
            return;
        };
        let Some(root) = page.source.root.clone() else {
            self.fail(&crate::i18n::tr("No configuration folder is available."));
            return;
        };
        self.run(move || integration_catalog::install(&entry, &root).map(|_| ()));
    }

    fn set_enabled(self: &Rc<Self>, enabled: bool) {
        let Some(page) = self.page.upgrade() else {
            return;
        };
        let Some(root) = page.source.root.clone() else {
            return;
        };
        let id = self.id.clone();
        self.run(move || integration_catalog::set_enabled(&root, &id, enabled));
    }

    fn remove(self: &Rc<Self>) {
        let Some(page) = self.page.upgrade() else {
            return;
        };
        let Some(root) = page.source.root.clone() else {
            return;
        };
        let id = self.id.clone();
        self.run(move || integration_catalog::remove(&root, &id));
    }

    fn run(self: &Rc<Self>, work: impl FnOnce() -> Result<(), String> + Send + 'static) {
        self.busy(true);
        self.message.set_visible(false);
        let context = self.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(work).await;
            let Some(page) = context.page.upgrade() else {
                return;
            };
            match result {
                Ok(Ok(())) => {
                    (page.source.reload)();
                    page.rebuild();
                }
                Ok(Err(error)) => {
                    context.fail(&rust_i18n::t!("The change failed: %{error}", error = error))
                }
                Err(_) => context.fail(&crate::i18n::tr("The change failed.")),
            }
        });
    }

    fn fail(&self, text: &str) {
        self.message.set_text(text);
        self.message.add_css_class("error");
        self.message.set_visible(true);
        self.busy(false);
    }
}

/// A text button whose accessible name says which entry it acts on. A label
/// child would otherwise name the button after its text alone.
fn named_button(label: &str, accessible: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(&gtk::Label::new(Some(&crate::i18n::tr(label)))));
    button.add_css_class("text-button");
    accessibility::set_label(&button, accessible);
    button
}

fn toggle_label(enabled: bool) -> String {
    crate::i18n::tr(if enabled { "Enabled" } else { "Disabled" })
}

/// A text button with a leading icon; the accessible name is the label.
fn icon_label_button(icon: &str, label: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.set_halign(gtk::Align::Center);
    let image = assets::text_icon(icon, assets::CHROME_ICON_PX);
    image.set_valign(gtk::Align::Center);
    content.append(&image);
    let text = crate::i18n::tr(label);
    content.append(&gtk::Label::new(Some(&text)));
    let button = gtk::Button::builder().child(&content).build();
    button.add_css_class("settings-action-button");
    accessibility::set_label(&button, &text);
    button
}

/// The entry's artwork, or its category icon, in a bordered tile.
fn logo_tile(icon: Option<&[u8]>, category: Category) -> gtk::Widget {
    let texture = icon.and_then(|bytes| gdk::Texture::from_bytes(&glib::Bytes::from(bytes)).ok());
    let image = match texture {
        Some(texture) => gtk::Image::from_paintable(Some(&texture)),
        None => assets::primary_icon(crate::ui::file_providers::category_icon(Some(category)), 20),
    };
    image.set_pixel_size(20);
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    image.set_hexpand(true);
    image.set_vexpand(true);
    let tile = gtk::Box::new(gtk::Orientation::Vertical, 0);
    tile.add_css_class("integration-logo");
    tile.set_size_request(36, 36);
    // Centering the icon must not stretch the tile itself.
    tile.set_hexpand(false);
    tile.set_vexpand(false);
    tile.set_valign(gtk::Align::Start);
    tile.append(&image);
    tile.upcast()
}

fn category_label(category: Category) -> &'static str {
    match category {
        Category::Cloud => "Cloud",
        Category::Devices => "Devices",
        Category::VersionControl => "Version control",
        Category::Other => "Other",
    }
}

/// A heading with a count beside it; the count is filled in by the caller.
fn counted_heading(text: &str) -> (gtk::Box, gtk::Label) {
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = gtk::Label::new(Some(&crate::i18n::tr(text)));
    label.set_xalign(0.0);
    // Kept out of Settings' label wrapping, which squeezes it beside the count.
    label.add_css_class("integration-heading");
    label.add_css_class("settings-control-label");
    heading.append(&label);
    let count = gtk::Label::new(None);
    count.add_css_class("integration-count");
    count.set_valign(gtk::Align::Center);
    heading.append(&count);
    (heading, count)
}

fn note(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    crate::ui::controls::keep_words_whole(&label);
    label.add_css_class("settings-option-description");
    label
}
