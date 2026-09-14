// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::mpsc::TryRecvError,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};

use crate::services::{SearchCoverage, SearchEvent, SearchHandle, SearchItem, index_trees};

const MAX_RESULT_UPDATES_PER_FRAME: usize = 8;

type SelectionCallback = Rc<dyn Fn(usize)>;

#[derive(Clone)]
pub(super) struct SearchDropdown {
    button: gtk::MenuButton,
    label: gtk::Label,
    selected: Rc<Cell<usize>>,
    checks: Rc<RefCell<Vec<gtk::Image>>>,
    default_title: String,
    options: Vec<String>,
    changed: Rc<RefCell<Option<SelectionCallback>>>,
}

impl SearchDropdown {
    fn new(default_title: &str, options: &[&str], selected_idx: usize) -> Self {
        let options_vec: Vec<String> = options.iter().map(|s| s.to_string()).collect();
        let selected_idx = selected_idx.min(options.len().saturating_sub(1));
        let initial_text = if selected_idx == 0 {
            default_title
        } else {
            options.get(selected_idx).copied().unwrap_or(default_title)
        };

        let menu_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        menu_box.add_css_class("column-menu");

        let popover = gtk::Popover::builder()
            .child(&menu_box)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .build();
        popover.add_css_class("column-popover");

        let current_label = gtk::Label::new(Some(initial_text));
        current_label.set_xalign(0.0);
        let chevron = crate::assets::chrome_icon(crate::assets::icons::ARROW_DOWN);
        chevron.add_css_class("search-criteria-chevron");

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content.set_valign(gtk::Align::Center);
        content.append(&current_label);
        content.append(&chevron);

        let button = gtk::MenuButton::builder()
            .child(&content)
            .has_frame(false)
            .popover(&popover)
            .valign(gtk::Align::Center)
            .build();
        button.set_cursor_from_name(Some("pointer"));
        button.add_css_class("search-criteria-dropdown");
        if selected_idx > 0 {
            button.add_css_class("active-filter");
        }

        let selected = Rc::new(Cell::new(selected_idx));
        let checks = Rc::new(RefCell::new(Vec::new()));
        let changed = Rc::new(RefCell::new(None::<SelectionCallback>));

        let dropdown = Self {
            button: button.clone(),
            label: current_label,
            selected: selected.clone(),
            checks: checks.clone(),
            default_title: default_title.to_string(),
            options: options_vec,
            changed: changed.clone(),
        };

        let default_title_str = default_title.to_string();

        for (index, &option_label) in options.iter().enumerate() {
            let (option, check) = super::controls::menu_option(option_label, index == selected_idx);
            checks.borrow_mut().push(check);

            let weak_btn = button.downgrade();
            let weak_label = dropdown.label.downgrade();
            let selected_cell = selected.clone();
            let checks_list = checks.clone();
            let changed_cb = changed.clone();
            let option_str = option_label.to_string();
            let title_str = default_title_str.clone();

            option.connect_clicked(move |_| {
                selected_cell.set(index);
                if let Some(lbl) = weak_label.upgrade() {
                    let display_text = if index == 0 { &title_str } else { &option_str };
                    lbl.set_text(display_text);
                }
                if let Some(btn) = weak_btn.upgrade() {
                    if index == 0 {
                        btn.remove_css_class("active-filter");
                    } else {
                        btn.add_css_class("active-filter");
                    }
                    btn.popdown();
                }
                for (i, c) in checks_list.borrow().iter().enumerate() {
                    c.set_visible(i == index);
                }
                if let Some(cb) = changed_cb.borrow().as_ref() {
                    cb(index);
                }
            });
            menu_box.append(&option);
        }

        dropdown
    }

    fn widget(&self) -> gtk::Widget {
        self.button.clone().upcast()
    }

    pub(crate) fn selected(&self) -> usize {
        self.selected.get()
    }

    pub(crate) fn set_selected(&self, index: usize) {
        let index = index.min(self.options.len().saturating_sub(1));
        self.selected.set(index);
        let display_text = if index == 0 {
            &self.default_title
        } else {
            self.options.get(index).unwrap_or(&self.default_title)
        };
        self.label.set_text(display_text);
        if index == 0 {
            self.button.remove_css_class("active-filter");
        } else {
            self.button.add_css_class("active-filter");
        }
        for (i, check) in self.checks.borrow().iter().enumerate() {
            check.set_visible(i == index);
        }
        if let Some(cb) = self.changed.borrow().as_ref() {
            cb(index);
        }
    }

    fn connect_changed<F: Fn(usize) + 'static>(&self, callback: F) {
        self.changed.replace(Some(Rc::new(callback)));
    }
}

fn criterion_row(
    title: &str,
    operator: &str,
    dropdown: &SearchDropdown,
    removable: bool,
) -> (gtk::Box, Option<gtk::Button>, gtk::Button) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("search-criterion-row");
    row.set_valign(gtk::Align::Center);

    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("search-criterion-property");
    title_label.set_xalign(0.0);
    row.append(&title_label);
    let operator = gtk::Label::new(Some(operator));
    operator.add_css_class("search-criterion-operator");
    row.append(&operator);
    row.append(&dropdown.widget());
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    row.append(&spacer);

    let remove = removable.then(|| {
        let button = gtk::Button::builder()
            .child(&crate::assets::primary_icon(
                crate::assets::icons::MINUS,
                13,
            ))
            .has_frame(false)
            .build();
        button.add_css_class("search-criterion-action");
        button.set_cursor_from_name(Some("pointer"));
        super::accessibility::set_label(&button, &format!("Remove {title} criterion"));
        row.append(&button);
        button
    });
    let add = gtk::Button::builder()
        .child(&crate::assets::primary_icon(crate::assets::icons::PLUS, 13))
        .has_frame(false)
        .build();
    add.add_css_class("search-criterion-action");
    add.set_cursor_from_name(Some("pointer"));
    super::accessibility::set_label(&add, "Add search criterion");
    row.append(&add);

    (row, remove, add)
}

fn update_criterion_actions(
    date_row: &gtk::Box,
    size_row: &gtk::Box,
    kind_add: &gtk::Button,
    date_add: &gtk::Button,
    size_add: &gtk::Button,
) {
    kind_add.set_visible(!date_row.is_visible() && !size_row.is_visible());
    date_add.set_visible(date_row.is_visible() && !size_row.is_visible());
    size_add.set_visible(size_row.is_visible());
}

#[derive(Clone)]
pub struct SearchDialog {
    pub(super) state: Rc<SearchState>,
}

pub(super) struct SearchState {
    // Keep the shared provider alive when this dialog is hosted without a browser window.
    _themes: Rc<super::theme::ThemeManager>,
    layer: gtk::Box,
    field: gtk::Entry,
    indexing_spinner: gtk::Spinner,
    pub(super) scope_folder: gtk::ToggleButton,
    pub(super) scope_all: gtk::ToggleButton,
    pub(super) kind_dropdown: SearchDropdown,
    pub(super) date_dropdown: SearchDropdown,
    pub(super) size_dropdown: SearchDropdown,
    pub(super) date_row: gtk::Box,
    pub(super) size_row: gtk::Box,
    kind_add: gtk::Button,
    date_add: gtk::Button,
    size_add: gtk::Button,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    results: gtk::Stack,
    status: gtk::Label,
    result_count: gtk::Label,
    truncated_hint: gtk::Label,
    visible_results: RefCell<Vec<SearchItem>>,
    positions: Rc<RefCell<HashMap<gtk::ListBoxRow, usize>>>,
    requested_thumbnails: RefCell<HashSet<PathBuf>>,
    rendered_query: RefCell<String>,
    search: RefCell<Option<SearchHandle>>,
    generation: Cell<u64>,
    interaction_revision: Cell<u64>,
    navigation_started: Cell<bool>,
    reconciling_results: Cell<bool>,
    activate: Rc<dyn Fn(SearchItem)>,
    dismiss: Rc<dyn Fn()>,
    roots: RefCell<Vec<PathBuf>>,
    scope_root: RefCell<Option<PathBuf>>,
    show_hidden: Cell<bool>,
    save_button: gtk::MenuButton,
}

impl SearchDialog {
    #[expect(
        deprecated,
        reason = "GTK 4.12 deprecated translate_coordinates and allocation without a replacement for click-in-bounds checks"
    )]
    pub fn new(activate: Rc<dyn Fn(SearchItem)>, dismiss: Rc<dyn Fn()>) -> Self {
        let themes = super::theme::ThemeManager::shared();
        let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        layer.add_css_class("search-backdrop");
        layer.add_css_class("app-modal-layer");
        layer.set_halign(gtk::Align::Fill);
        layer.set_valign(gtk::Align::Fill);
        layer.set_hexpand(true);
        layer.set_vexpand(true);
        layer.set_focusable(true);
        layer.set_visible(false);

        let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        panel.add_css_class("search-dialog");
        panel.set_halign(gtk::Align::Center);
        panel.set_valign(gtk::Align::Center);
        panel.set_size_request(760, 480);
        panel.set_vexpand(false);
        panel.set_overflow(gtk::Overflow::Hidden);

        let search_bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        search_bar.add_css_class("search-bar");
        search_bar.append(&crate::assets::primary_icon(
            crate::assets::icons::SEARCH,
            20,
        ));
        let field = gtk::Entry::builder()
            .placeholder_text("Search files and folders…")
            .hexpand(true)
            .build();
        field.add_css_class("search-field");
        search_bar.append(&field);
        let indexing_spinner = gtk::Spinner::new();
        indexing_spinner.add_css_class("search-indexing-spinner");
        indexing_spinner.set_tooltip_text(Some("Indexing files…"));
        indexing_spinner.set_valign(gtk::Align::Center);
        indexing_spinner.set_visible(false);
        search_bar.append(&indexing_spinner);
        panel.append(&search_bar);

        let scope_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        scope_bar.add_css_class("search-scope-bar");
        scope_bar.set_valign(gtk::Align::Center);
        let scope_label = gtk::Label::new(Some("Search:"));
        scope_label.add_css_class("search-scope-label");
        scope_bar.append(&scope_label);

        let scope_switcher = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        scope_switcher.add_css_class("search-scope-switcher");
        let scope_folder = gtk::ToggleButton::with_label("Current Folder");
        scope_folder.add_css_class("search-scope-option");
        scope_folder.set_tooltip_text(Some("Search only the current folder"));
        let scope_all = gtk::ToggleButton::with_label("This Computer");
        scope_all.add_css_class("search-scope-option");
        scope_all.set_group(Some(&scope_folder));
        scope_all.set_active(true);
        scope_all.set_tooltip_text(Some("Search Home and mounted local drives"));
        scope_switcher.append(&scope_folder);
        scope_switcher.append(&scope_all);
        scope_bar.append(&scope_switcher);
        panel.append(&scope_bar);

        let save_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        save_box.add_css_class("smart-folder-save-box");
        let save_heading = gtk::Label::new(Some("SAVE AS SMART FOLDER"));
        save_heading.add_css_class("menu-heading");
        save_heading.set_xalign(0.0);
        let save_name_entry = gtk::Entry::builder()
            .placeholder_text("Smart Folder Name…")
            .build();
        save_name_entry.add_css_class("form-control");
        let save_summary = gtk::Label::new(None);
        save_summary.add_css_class("smart-folder-save-summary");
        save_summary.set_wrap(true);
        save_summary.set_xalign(0.0);
        let save_confirm = gtk::Button::builder()
            .label("Save to Sidebar")
            .css_classes(["save-smart-folder-action"])
            .build();
        save_box.append(&save_heading);
        save_box.append(&save_name_entry);
        save_box.append(&save_summary);
        save_box.append(&save_confirm);

        let save_popover = gtk::Popover::builder()
            .child(&save_box)
            .autohide(true)
            .has_arrow(true)
            .position(gtk::PositionType::Bottom)
            .build();

        let save_button = gtk::MenuButton::builder()
            .popover(&save_popover)
            .has_frame(false)
            .tooltip_text("Save this search as a Smart Folder in the sidebar")
            .css_classes(["search-save-button"])
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        save_button.set_cursor_from_name(Some("pointer"));
        let save_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        save_content.set_valign(gtk::Align::Center);
        save_content.append(&crate::assets::primary_icon(
            crate::assets::icons::FOLDER_PLUS,
            14,
        ));
        save_content.append(&gtk::Label::new(Some("Save")));
        save_button.set_child(Some(&save_content));

        let scope_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        scope_spacer.set_hexpand(true);
        scope_bar.append(&scope_spacer);
        scope_bar.append(&save_button);

        let criteria_bar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        criteria_bar.add_css_class("search-criteria-bar");

        let kind_dropdown = SearchDropdown::new(
            "Any Kind",
            &[
                "Any Kind",
                "Documents",
                "Images",
                "Audio",
                "Videos",
                "Archives",
                "Code",
                "Folders",
            ],
            0,
        );
        let date_dropdown = SearchDropdown::new(
            "Any Time",
            &[
                "Any Time",
                "Past 24 Hours",
                "Past 7 Days",
                "Past 30 Days",
                "Past Year",
            ],
            0,
        );
        let size_dropdown = SearchDropdown::new(
            "Any Size",
            &[
                "Any Size",
                "Larger than 1 MB",
                "Larger than 10 MB",
                "Larger than 100 MB",
                "Larger than 1 GB",
                "Smaller than 10 MB",
            ],
            0,
        );

        let (kind_row, _, kind_add) = criterion_row("Kind", "is", &kind_dropdown, false);
        let (date_row, date_remove, date_add) =
            criterion_row("Date Modified", "is within", &date_dropdown, true);
        let (size_row, size_remove, size_add) =
            criterion_row("File Size", "is", &size_dropdown, true);
        date_row.set_visible(false);
        size_row.set_visible(false);
        date_add.set_visible(false);
        size_add.set_visible(false);
        criteria_bar.append(&kind_row);
        criteria_bar.append(&date_row);
        criteria_bar.append(&size_row);
        panel.append(&criteria_bar);

        let status = gtk::Label::new(Some("Type to search Home and mounted local drives"));
        status.add_css_class("search-status");
        status.set_wrap(true);

        let list = gtk::ListBox::new();
        list.add_css_class("search-results");
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_activate_on_single_click(true);
        let positions = Rc::new(RefCell::new(HashMap::new()));
        let sorted_positions = positions.clone();
        list.set_sort_func(move |left, right| {
            let positions = sorted_positions.borrow();
            positions.get(left).cmp(&positions.get(right)).into()
        });
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .child(&list)
            .build();
        scroller.add_css_class("search-results-scroll");
        let results = gtk::Stack::new();
        results.add_css_class("search-results-stack");
        results.set_size_request(-1, 340);
        results.add_named(&status, Some("status"));
        results.add_named(&scroller, Some("results"));
        results.set_visible_child_name("status");
        panel.append(&results);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        footer.add_css_class("search-footer");
        let result_count = gtk::Label::new(None);
        result_count.add_css_class("search-result-count");
        result_count.set_visible(false);
        footer.append(&result_count);
        let navigation = gtk::Label::new(Some("↑↓  navigate"));
        let open = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        open.set_valign(gtk::Align::Center);
        open.append(&crate::assets::primary_icon(
            crate::assets::icons::CORNER_DOWN_LEFT,
            13,
        ));
        open.append(&gtk::Label::new(Some("open")));
        navigation.add_css_class("search-hint");
        open.add_css_class("search-hint");
        footer.append(&navigation);
        footer.append(&open);
        let truncated_hint = gtk::Label::new(None);
        truncated_hint.set_wrap(true);
        truncated_hint.set_max_width_chars(58);
        truncated_hint.add_css_class("search-hint");
        truncated_hint.add_css_class("search-hint-warning");
        truncated_hint.set_hexpand(true);
        truncated_hint.set_halign(gtk::Align::End);
        truncated_hint.set_visible(false);
        footer.append(&truncated_hint);
        panel.append(&footer);
        super::modal::layout::install(&layer, &panel);

        let state = Rc::new(SearchState {
            _themes: themes,
            layer,
            field,
            indexing_spinner,
            scope_folder,
            scope_all,
            kind_dropdown,
            date_dropdown,
            size_dropdown,
            date_row,
            size_row,
            kind_add,
            date_add,
            size_add,
            list,
            scroller,
            results,
            status,
            result_count,
            truncated_hint,
            visible_results: RefCell::new(Vec::new()),
            positions,
            requested_thumbnails: RefCell::new(HashSet::new()),
            rendered_query: RefCell::new(String::new()),
            search: RefCell::new(None),
            generation: Cell::new(0),
            interaction_revision: Cell::new(0),
            navigation_started: Cell::new(false),
            reconciling_results: Cell::new(false),
            activate,
            dismiss,
            roots: RefCell::new(Vec::new()),
            scope_root: RefCell::new(None),
            show_hidden: Cell::new(false),
            save_button: save_button.clone(),
        });

        for add in [&state.kind_add, &state.date_add, &state.size_add] {
            let weak = Rc::downgrade(&state);
            add.connect_clicked(move |_| {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                if !state.date_row.is_visible() {
                    state.date_row.set_visible(true);
                } else if !state.size_row.is_visible() {
                    state.size_row.set_visible(true);
                }
                update_criterion_actions(
                    &state.date_row,
                    &state.size_row,
                    &state.kind_add,
                    &state.date_add,
                    &state.size_add,
                );
            });
        }
        let date_remove = date_remove.expect("date criterion is removable");
        let date_remove_state = Rc::downgrade(&state);
        date_remove.connect_clicked(move |_| {
            if let Some(state) = date_remove_state.upgrade() {
                state.date_row.set_visible(false);
                state.date_dropdown.set_selected(0);
                update_criterion_actions(
                    &state.date_row,
                    &state.size_row,
                    &state.kind_add,
                    &state.date_add,
                    &state.size_add,
                );
            }
        });
        let size_remove = size_remove.expect("size criterion is removable");
        let size_remove_state = Rc::downgrade(&state);
        size_remove.connect_clicked(move |_| {
            if let Some(state) = size_remove_state.upgrade() {
                state.size_row.set_visible(false);
                state.size_dropdown.set_selected(0);
                update_criterion_actions(
                    &state.date_row,
                    &state.size_row,
                    &state.kind_add,
                    &state.date_add,
                    &state.size_add,
                );
            }
        });

        let entry_for_map = save_name_entry.clone();
        let save_state_for_map = Rc::downgrade(&state);
        save_popover.connect_map(move |_| {
            let Some(state) = save_state_for_map.upgrade() else {
                return;
            };
            let query = state.field.text().to_string();
            let rules = active_rules(&state);
            let default_name = if !query.trim().is_empty() {
                query.trim().to_string()
            } else if let Some(rule) = rules.first() {
                match rule {
                    crate::model::SmartQueryRule::Kind(k) => k.label().to_string(),
                    crate::model::SmartQueryRule::DateModified(d) => {
                        format!("Recent ({})", d.label())
                    }
                    crate::model::SmartQueryRule::FileSize(s) => format!("Files ({})", s.label()),
                    crate::model::SmartQueryRule::NameContains(n) => n.clone(),
                }
            } else {
                "Smart Search".to_string()
            };
            entry_for_map.set_text(&default_name);
            entry_for_map.select_region(0, -1);
            entry_for_map.grab_focus();
            let scope = if state.scope_folder.is_active() {
                state.scope_folder.label().unwrap_or_default().to_string()
            } else {
                "This Computer".to_string()
            };
            let criteria = if rules.is_empty() {
                format!("Name contains \u{201c}{}\u{201d}", query.trim())
            } else {
                rules
                    .iter()
                    .map(crate::model::SmartQueryRule::summary)
                    .collect::<Vec<_>>()
                    .join(" · ")
            };
            save_summary.set_text(&format!("Searches {scope} automatically\n{criteria}"));
        });

        let save_state_for_confirm = Rc::downgrade(&state);
        let popover_for_confirm = save_popover.clone();
        let entry_for_confirm = save_name_entry.clone();
        let do_save = move || {
            let Some(state) = save_state_for_confirm.upgrade() else {
                return;
            };
            let mut name = entry_for_confirm.text().trim().to_string();
            if name.is_empty() {
                name = "Smart Folder".to_string();
            }
            let query = state.field.text().to_string();
            let rules = active_rules(&state);
            let id = format!(
                "smart-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0)
            );
            let roots = if state.scope_folder.is_active() {
                active_search_roots(&state)
            } else {
                Vec::new()
            };
            let show_hidden = state.show_hidden.get();
            let definition = super::theme::SmartFolderDef {
                id,
                name,
                query,
                rules,
                roots,
                show_hidden,
            };
            super::theme::ThemeManager::shared().add_smart_folder(definition);
            popover_for_confirm.popdown();
            hide(&state);
        };
        let do_save_rc = Rc::new(do_save);
        let do_save_btn = do_save_rc.clone();
        save_confirm.connect_clicked(move |_| do_save_btn());
        let do_save_enter = do_save_rc.clone();
        save_name_entry.connect_activate(move |_| do_save_enter());

        let folder_scope_state = Rc::downgrade(&state);
        state.scope_folder.connect_toggled(move |button| {
            if button.is_active()
                && let Some(state) = folder_scope_state.upgrade()
                && state.layer.is_visible()
            {
                start_indexing(&state);
            }
        });
        let all_scope_state = Rc::downgrade(&state);
        state.scope_all.connect_toggled(move |button| {
            if button.is_active()
                && let Some(state) = all_scope_state.upgrade()
                && state.layer.is_visible()
            {
                start_indexing(&state);
            }
        });

        let changed = Rc::downgrade(&state);
        state.field.connect_changed(move |_| {
            if let Some(state) = changed.upgrade() {
                begin_query(&state);
            }
        });
        let changed_kind = Rc::downgrade(&state);
        state.kind_dropdown.connect_changed(move |_| {
            if let Some(state) = changed_kind.upgrade() {
                begin_query(&state);
            }
        });
        let changed_date = Rc::downgrade(&state);
        state.date_dropdown.connect_changed(move |_| {
            if let Some(state) = changed_date.upgrade() {
                begin_query(&state);
            }
        });
        let changed_size = Rc::downgrade(&state);
        state.size_dropdown.connect_changed(move |_| {
            if let Some(state) = changed_size.upgrade() {
                begin_query(&state);
            }
        });
        let activated = Rc::downgrade(&state);
        state.list.connect_row_activated(move |_, row| {
            if let Some(state) = activated.upgrade() {
                activate_position(&state, row.index());
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let keyed = Rc::downgrade(&state);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(state) = keyed.upgrade() else {
                return glib::Propagation::Proceed;
            };
            record_interaction(&state);
            if key == gdk::Key::Escape {
                hide(&state);
                return glib::Propagation::Stop;
            }
            if modifiers.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK,
            ) {
                return glib::Propagation::Proceed;
            }
            if matches!(key, gdk::Key::Down | gdk::Key::Up)
                && !modifiers.contains(gdk::ModifierType::SHIFT_MASK)
            {
                move_selection(&state, if key == gdk::Key::Down { 1 } else { -1 });
                return glib::Propagation::Stop;
            }
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) && activate_selected(&state) {
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        state.layer.add_controller(keys);

        let click_state = Rc::downgrade(&state);
        let click_panel = panel.clone();
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(move |_, _, x, y| {
            let Some(state) = click_state.upgrade() else {
                return;
            };
            record_interaction(&state);
            let on_panel = click_panel
                .translate_coordinates(&state.layer, 0.0, 0.0)
                .is_some_and(|(px, py)| {
                    let alloc = click_panel.allocation();
                    x >= px
                        && x < px + alloc.width() as f64
                        && y >= py
                        && y < py + alloc.height() as f64
                });
            if !on_panel {
                hide(&state);
            }
        });
        state.layer.add_controller(click);
        let wheel_state = Rc::downgrade(&state);
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
        wheel.connect_scroll(move |_, _, _| {
            if let Some(state) = wheel_state.upgrade() {
                record_interaction(&state);
            }
            glib::Propagation::Proceed
        });
        state.layer.add_controller(wheel);
        let adjustment = state.scroller.vadjustment();
        let changed = Rc::downgrade(&state);
        adjustment.connect_changed(move |_| {
            if let Some(state) = changed.upgrade()
                && !state.reconciling_results.get()
            {
                refresh_visible_thumbnails(&state);
            }
        });
        let scrolled = Rc::downgrade(&state);
        adjustment.connect_value_changed(move |_| {
            if let Some(state) = scrolled.upgrade()
                && !state.reconciling_results.get()
            {
                refresh_visible_thumbnails(&state);
            }
        });

        Self { state }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.state.layer.clone().upcast()
    }

    pub fn show(
        &self,
        roots: Vec<PathBuf>,
        current_folder: Option<(PathBuf, String)>,
        show_hidden: bool,
    ) {
        self.state.roots.replace(roots);
        let (scope_root, scope_name) = current_folder
            .map(|(path, name)| (Some(path), name))
            .unwrap_or_else(|| (None, "Current Folder".to_string()));
        self.state.scope_root.replace(scope_root);
        self.state.scope_folder.set_label(&scope_name);
        self.state
            .scope_folder
            .set_sensitive(self.state.scope_root.borrow().is_some());
        self.state.show_hidden.set(show_hidden);
        self.state.scope_all.set_active(true);
        self.state.kind_dropdown.set_selected(0);
        self.state.date_dropdown.set_selected(0);
        self.state.size_dropdown.set_selected(0);
        self.state.date_row.set_visible(false);
        self.state.size_row.set_visible(false);
        update_criterion_actions(
            &self.state.date_row,
            &self.state.size_row,
            &self.state.kind_add,
            &self.state.date_add,
            &self.state.size_add,
        );
        self.state.save_button.set_visible(false);
        self.state.result_count.set_visible(false);
        self.state.field.set_text("");
        self.state.layer.set_visible(true);
        super::browser::animate_in(&self.state.layer);
        start_indexing(&self.state);
        self.state.field.grab_focus_without_selecting();
    }

    pub fn hide(&self) {
        hide(&self.state);
    }

    pub fn is_visible(&self) -> bool {
        self.state.layer.is_visible()
    }
}

fn active_search_roots(state: &SearchState) -> Vec<PathBuf> {
    if state.scope_folder.is_active() {
        state.scope_root.borrow().clone().into_iter().collect()
    } else {
        state.roots.borrow().clone()
    }
}

fn start_indexing(state: &Rc<SearchState>) {
    state.generation.set(state.generation.get() + 1);
    let generation = state.generation.get();
    state.search.borrow_mut().take();
    clear_results(state);
    state.results.set_visible_child_name("status");
    state.truncated_hint.set_visible(false);
    state.result_count.set_visible(false);

    let roots = active_search_roots(state);
    let locations = roots
        .iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    state.field.set_tooltip_text(Some(&format!(
        "Search locations:\n{locations}\nRemote shares are not included."
    )));
    state.field.set_sensitive(!roots.is_empty());
    state.status.set_visible(true);
    let has_criteria = !state.field.text().trim().is_empty() || !active_rules(state).is_empty();
    if roots.is_empty() {
        state
            .status
            .set_text("No local search locations available.");
    } else if has_criteria {
        state.status.set_text("Searching…");
    } else if state.scope_folder.is_active() {
        state.status.set_text(&format!(
            "Type to search {}",
            state.scope_folder.label().unwrap_or_default()
        ));
    } else {
        state
            .status
            .set_text("Type to search Home and mounted local drives");
    }

    if roots.is_empty() {
        state.indexing_spinner.stop();
        state.indexing_spinner.set_visible(false);
        state.layer.grab_focus();
        return;
    }

    state.indexing_spinner.set_visible(true);
    state.indexing_spinner.start();
    let (handle, receiver) = index_trees(roots, state.show_hidden.get());
    if has_criteria {
        handle.smart_query(&state.field.text(), active_rules(state));
    }
    state.search.replace(Some(handle));
    let weak = Rc::downgrade(state);
    let _poll = glib::timeout_add_local(Duration::from_millis(16), move || {
        let Some(state) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !state.layer.is_visible() || state.generation.get() != generation {
            return glib::ControlFlow::Break;
        }
        let mut latest = None;
        for _ in 0..MAX_RESULT_UPDATES_PER_FRAME {
            match receiver.try_recv() {
                Ok(event) => latest = Some(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return glib::ControlFlow::Break,
            }
        }
        if let Some(SearchEvent::Results {
            query,
            rules,
            items,
            indexing,
            coverage,
        }) = latest
        {
            if indexing {
                state.indexing_spinner.set_visible(true);
                state.indexing_spinner.start();
            } else {
                state.indexing_spinner.stop();
                state.indexing_spinner.set_visible(false);
            }
            let current_rules = active_rules(&state);
            let is_current = query == state.field.text().trim() && rules == current_rules;
            if is_current {
                state.truncated_hint.set_text(&coverage.message());
                state.truncated_hint.set_visible(coverage.is_partial());
            }
            if is_current && (!query.is_empty() || !rules.is_empty()) {
                render_results(&state, items, indexing, coverage);
            }
        }
        glib::ControlFlow::Continue
    });
}

fn active_rules(state: &SearchState) -> Vec<crate::model::SmartQueryRule> {
    use crate::model::{DateConstraint, FileCategory, SizeConstraint, SmartQueryRule};
    let mut rules = Vec::new();
    match state.kind_dropdown.selected() {
        1 => rules.push(SmartQueryRule::Kind(FileCategory::Document)),
        2 => rules.push(SmartQueryRule::Kind(FileCategory::Image)),
        3 => rules.push(SmartQueryRule::Kind(FileCategory::Audio)),
        4 => rules.push(SmartQueryRule::Kind(FileCategory::Video)),
        5 => rules.push(SmartQueryRule::Kind(FileCategory::Archive)),
        6 => rules.push(SmartQueryRule::Kind(FileCategory::Code)),
        7 => rules.push(SmartQueryRule::Kind(FileCategory::Folder)),
        _ => {}
    }
    if state.date_row.is_visible() {
        match state.date_dropdown.selected() {
            1 => rules.push(SmartQueryRule::DateModified(
                DateConstraint::WithinPastDays(1),
            )),
            2 => rules.push(SmartQueryRule::DateModified(
                DateConstraint::WithinPastDays(7),
            )),
            3 => rules.push(SmartQueryRule::DateModified(
                DateConstraint::WithinPastDays(30),
            )),
            4 => rules.push(SmartQueryRule::DateModified(
                DateConstraint::WithinPastDays(365),
            )),
            _ => {}
        }
    }
    if state.size_row.is_visible() {
        match state.size_dropdown.selected() {
            1 => rules.push(SmartQueryRule::FileSize(SizeConstraint::GreaterThan(
                1_000_000,
            ))),
            2 => rules.push(SmartQueryRule::FileSize(SizeConstraint::GreaterThan(
                10_000_000,
            ))),
            3 => rules.push(SmartQueryRule::FileSize(SizeConstraint::GreaterThan(
                100_000_000,
            ))),
            4 => rules.push(SmartQueryRule::FileSize(SizeConstraint::GreaterThan(
                1_000_000_000,
            ))),
            5 => rules.push(SmartQueryRule::FileSize(SizeConstraint::LessThan(
                10_000_000,
            ))),
            _ => {}
        }
    }
    rules
}

fn begin_query(state: &Rc<SearchState>) {
    record_interaction(state);
    state.navigation_started.set(false);
    let query = state.field.text().to_string();
    let rules = active_rules(state);
    let has_criteria = !query.trim().is_empty() || !rules.is_empty();
    state.save_button.set_visible(has_criteria);
    state.result_count.set_visible(has_criteria);
    if !has_criteria {
        clear_results(state);
        state.result_count.set_visible(false);
        state.results.set_visible_child_name("status");
        if state.scope_folder.is_active() {
            state.status.set_text(&format!(
                "Type to search {}\nOr select criteria above to filter files",
                state.scope_folder.label().unwrap_or_default()
            ));
        } else {
            state.status.set_text(
                "Type to search Home and mounted local drives\nOr select criteria above to filter files",
            );
        }
    } else if state.visible_results.borrow().is_empty() {
        state.result_count.set_text("Searching…");
        state.results.set_visible_child_name("status");
        state.status.set_text("Searching…");
    }
    if let Some(search) = state.search.borrow().as_ref() {
        search.smart_query(&query, rules);
    }
}

fn render_results(
    state: &Rc<SearchState>,
    results: Vec<SearchItem>,
    indexing: bool,
    coverage: SearchCoverage,
) {
    let query = state.field.text().trim().to_owned();
    let query_changed = state.rendered_query.borrow().as_str() != query;
    let old_items = state.visible_results.borrow().clone();
    let results_changed = old_items != results;
    let selected_index = state.list.selected_row().map(|row| row.index());
    let selected_path = selected_index
        .and_then(|position| usize::try_from(position).ok())
        .and_then(|position| old_items.get(position))
        .map(|item| item.path.clone());
    let focused = state.list.root().and_then(|root| root.focus());
    let focused_path = old_items.iter().enumerate().find_map(|(position, item)| {
        let row = state.list.row_at_index(position as i32)?;
        focused
            .as_ref()
            .filter(|focused| **focused == row || focused.is_ancestor(&row))
            .map(|_| item.path.clone())
    });
    let had_result_focus = focused_path.is_some();
    let scroll_position = state.scroller.vadjustment().value();

    if results_changed {
        state.reconciling_results.set(true);
        let mut rows = old_items
            .iter()
            .enumerate()
            .filter_map(|(position, item)| {
                state
                    .list
                    .row_at_index(position as i32)
                    .map(|row| (item.path.clone(), (item.clone(), row)))
            })
            .collect::<HashMap<_, _>>();
        let mut ordered_rows = Vec::with_capacity(results.len());
        let mut requested = state.requested_thumbnails.borrow_mut();

        for item in &results {
            let row = if let Some((previous, row)) = rows.remove(&item.path) {
                if previous == *item {
                    row
                } else {
                    requested.remove(&item.path);
                    super::thumbnail::cancel_thumbnails_in(row.upcast_ref());
                    state.list.remove(&row);
                    let row = result_row(item);
                    state.list.append(&row);
                    row
                }
            } else {
                let row = result_row(item);
                state.list.append(&row);
                row
            };
            ordered_rows.push(row);
        }
        for (path, (_, row)) in rows {
            requested.remove(&path);
            super::thumbnail::cancel_thumbnails_in(row.upcast_ref());
            state.list.remove(&row);
        }
        drop(requested);

        state.positions.replace(
            ordered_rows
                .into_iter()
                .enumerate()
                .map(|(position, row)| (row, position))
                .collect(),
        );
        state.visible_results.replace(results);
        state.list.invalidate_sort();
        state.reconciling_results.set(false);
    }

    state.rendered_query.replace(query);
    let result_len = state.visible_results.borrow().len();
    let has_results = result_len > 0;
    state.result_count.set_visible(true);
    let count_text = match (result_len, indexing) {
        (0, true) => "Searching…".to_string(),
        (0, false) => "No results".to_string(),
        (1, _) => "1 item".to_string(),
        (count, _) => format!("{count} items"),
    };
    state.result_count.set_text(&count_text);
    state.truncated_hint.set_text(&coverage.message());
    state.truncated_hint.set_visible(coverage.is_partial());
    state
        .results
        .set_visible_child_name(if has_results { "results" } else { "status" });
    if has_results && (results_changed || query_changed) {
        let items = state.visible_results.borrow();
        let restored = if query_changed {
            0
        } else {
            selected_path
                .as_ref()
                .and_then(|path| items.iter().position(|item| &item.path == path))
                .or_else(|| {
                    selected_index.map(|position| {
                        usize::try_from(position)
                            .unwrap_or_default()
                            .min(items.len() - 1)
                    })
                })
                .unwrap_or(0)
        };
        drop(items);
        state
            .list
            .select_row(state.list.row_at_index(restored as i32).as_ref());
    } else if !has_results {
        state.status.set_text(if indexing {
            "Searching…"
        } else {
            "No matching files or folders"
        });
    }

    if results_changed
        && had_result_focus
        && focused
            .as_ref()
            .is_some_and(|focused| focused.root().is_none())
    {
        let focused_position = focused_path.and_then(|path| {
            state
                .visible_results
                .borrow()
                .iter()
                .position(|item| item.path == path)
        });
        if let Some(row) = focused_position
            .and_then(|position| state.list.row_at_index(position as i32))
            .or_else(|| state.list.selected_row())
        {
            row.grab_focus();
        } else {
            state.field.grab_focus_without_selecting();
        }
    }

    if results_changed {
        record_interaction(state);
        let revision = state.interaction_revision.get();
        let weak = Rc::downgrade(state);
        state.list.add_tick_callback(move |_, _| {
            let weak = weak.clone();
            // Reordered rows keep their old allocation until this frame's layout.
            glib::idle_add_local_once(move || {
                if let Some(state) = weak.upgrade() {
                    if state.layer.is_visible() && state.interaction_revision.get() == revision {
                        if state.navigation_started.get() {
                            if let Some(row) = state.list.selected_row() {
                                scroll_row_into_view(&state, &row);
                            }
                        } else {
                            state.scroller.vadjustment().set_value(scroll_position);
                        }
                    }
                    refresh_visible_thumbnails(&state);
                }
            });
            glib::ControlFlow::Break
        });
    }
}

fn result_row(item: &SearchItem) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("search-result");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let icon = super::thumbnail::ThumbnailSlot::new(19);
    icon.add_css_class("search-result-thumbnail");
    let fallback = if item.is_directory {
        crate::assets::icons::FOLDER
    } else {
        crate::assets::icons::DOCUMENTS
    };
    super::thumbnail::show_customized_icon(&icon, &item.path, fallback, 19);
    content.append(&icon);
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let name = gtk::Label::new(Some(&item.name));
    name.add_css_class("search-result-name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let full_path = item.path.to_string_lossy();
    row.set_tooltip_text(Some(&full_path));
    let path = gtk::Label::new(Some(&full_path));
    path.add_css_class("search-result-path");
    path.set_xalign(0.0);
    path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    labels.append(&name);
    labels.append(&path);
    content.append(&labels);
    row.set_child(Some(&content));
    row
}

fn refresh_visible_thumbnails(state: &SearchState) {
    let adjustment = state.scroller.vadjustment();
    let viewport_top = adjustment.value();
    let viewport_height = adjustment.page_size();
    let changes = {
        let items = state.visible_results.borrow();
        let mut requested = state.requested_thumbnails.borrow_mut();
        let mut changes = Vec::new();
        for (position, item) in items.iter().enumerate() {
            let Some(row) = i32::try_from(position)
                .ok()
                .and_then(|position| state.list.row_at_index(position))
            else {
                continue;
            };
            let Some(bounds) = row.compute_bounds(&state.list) else {
                continue;
            };
            let visible = intersects_viewport(
                f64::from(bounds.y()),
                f64::from(bounds.height()),
                viewport_top,
                viewport_height,
            );
            let Some(image) = row
                .child()
                .and_then(|content| content.first_child())
                .and_then(|child| child.downcast::<super::thumbnail::ThumbnailSlot>().ok())
            else {
                continue;
            };
            if visible && requested.insert(item.path.clone()) {
                changes.push((image, Some(item.path.clone()), item.is_directory));
            } else if !visible && requested.remove(&item.path) {
                changes.push((image, None, item.is_directory));
            }
        }
        changes
    };
    for (image, path, is_directory) in changes {
        let fallback = if is_directory {
            crate::assets::icons::FOLDER
        } else {
            crate::assets::icons::DOCUMENTS
        };
        if let Some(path) = path {
            super::thumbnail::set_thumbnail_or_icon_for_path(&image, &path, fallback, 19, 32);
        } else {
            super::thumbnail::show_fallback_icon(&image, fallback, 19);
        }
    }
}

fn intersects_viewport(
    row_top: f64,
    row_height: f64,
    viewport_top: f64,
    viewport_height: f64,
) -> bool {
    row_top < viewport_top + viewport_height && row_top + row_height > viewport_top
}

fn contains_keyboard_focus(widget: &gtk::Widget) -> bool {
    widget
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| gtk::prelude::GtkWindowExt::focus(&window))
        .is_some_and(|focused| focused == *widget || focused.is_ancestor(widget))
}

fn record_interaction(state: &SearchState) {
    state
        .interaction_revision
        .set(state.interaction_revision.get() + 1);
}

fn move_selection(state: &SearchState, direction: i32) {
    record_interaction(state);
    if !contains_keyboard_focus(state.field.upcast_ref()) {
        state.field.grab_focus_without_selecting();
    }
    let count = state.visible_results.borrow().len() as i32;
    if count == 0 {
        return;
    }

    state.navigation_started.set(true);
    let next = state
        .list
        .selected_row()
        .map_or(0, |row| (row.index() + direction).clamp(0, count - 1));
    if let Some(row) = state.list.row_at_index(next) {
        state.list.select_row(Some(&row));
        scroll_row_into_view(state, &row);
    }
}

fn scroll_row_into_view(state: &SearchState, row: &gtk::ListBoxRow) {
    let Some(bounds) = row.compute_bounds(&state.list) else {
        return;
    };
    let adjustment = state.scroller.vadjustment();
    let viewport_top = adjustment.value();
    let viewport_bottom = viewport_top + adjustment.page_size();
    let row_top = f64::from(bounds.y());
    let row_bottom = row_top + f64::from(bounds.height());
    if row_top < viewport_top {
        adjustment.set_value(row_top);
    } else if row_bottom > viewport_bottom {
        adjustment.set_value(row_bottom - adjustment.page_size());
    }
}

fn activate_selected(state: &Rc<SearchState>) -> bool {
    let Some(row) = state.list.selected_row() else {
        return false;
    };
    activate_position(state, row.index());
    true
}

fn activate_position(state: &Rc<SearchState>, position: i32) {
    let Some(item) = usize::try_from(position)
        .ok()
        .and_then(|position| state.visible_results.borrow().get(position).cloned())
    else {
        return;
    };
    hide(state);
    (state.activate)(item);
}

fn hide(state: &SearchState) {
    state.generation.set(state.generation.get() + 1);
    record_interaction(state);
    state.search.borrow_mut().take();
    clear_results(state);
    state.truncated_hint.set_visible(false);
    state.indexing_spinner.stop();
    state.indexing_spinner.set_visible(false);
    if state.layer.has_css_class("dismissing") {
        return;
    }
    state.layer.add_css_class("dismissing");
    state.layer.set_sensitive(false);
    let layer = state.layer.clone();
    let dismiss = state.dismiss.clone();
    super::browser::animate_out(&state.layer, move || {
        layer.set_visible(false);
        layer.remove_css_class("dismissing");
        layer.set_sensitive(true);
        dismiss();
    });
}

fn clear_results(state: &SearchState) {
    state.navigation_started.set(false);
    state.reconciling_results.set(true);
    state.visible_results.borrow_mut().clear();
    state.rendered_query.borrow_mut().clear();
    state.positions.borrow_mut().clear();
    state.requested_thumbnails.borrow_mut().clear();
    state.scroller.vadjustment().set_value(0.0);
    while let Some(child) = state.list.first_child() {
        super::thumbnail::cancel_thumbnails_in(&child);
        state.list.remove(&child);
    }
    state.reconciling_results.set(false);
}

#[cfg(test)]
mod tests;
