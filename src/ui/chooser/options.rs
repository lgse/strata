// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone)]
pub(super) struct PortalFilter {
    pub(super) portal: FileFilter,
    pub(super) native: gtk::FileFilter,
}

pub(super) enum ChoiceControl {
    Boolean {
        id: String,
        check: gtk::CheckButton,
    },
    Select {
        id: String,
        values: Vec<String>,
        dropdown: ChooserDropdown,
    },
}

pub(super) type SelectionChanged = Box<dyn Fn(usize)>;

pub(super) const DROPDOWN_EDGE_MARGIN: i32 = 24;
pub(super) const MIN_DROPDOWN_CONTENT_HEIGHT: i32 = 120;

// Popovers use separate surfaces, so the window does not constrain their content height.
pub(super) fn dropdown_placement(
    available_height: i32,
    anchor_top: i32,
    anchor_bottom: i32,
) -> (gtk::PositionType, i32) {
    let below = available_height.saturating_sub(anchor_bottom).max(0);
    let above = anchor_top.max(0);
    let (position, room) = if below >= above {
        (gtk::PositionType::Bottom, below)
    } else {
        (gtk::PositionType::Top, above)
    };
    (
        position,
        room.saturating_sub(DROPDOWN_EDGE_MARGIN)
            .max(MIN_DROPDOWN_CONTENT_HEIGHT),
    )
}

pub(super) struct ChooserDropdown {
    pub(super) button: gtk::MenuButton,
    pub(super) popover: gtk::Popover,
    pub(super) selected: Rc<Cell<usize>>,
    pub(super) changed: Rc<RefCell<Option<SelectionChanged>>>,
}

impl ChooserDropdown {
    pub(super) fn new(labels: &[&str], selected: usize) -> Self {
        let selected = selected.min(labels.len().saturating_sub(1));
        let current = labels.get(selected).copied().unwrap_or_default();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        content.add_css_class("column-menu");
        let scroll = gtk::ScrolledWindow::builder()
            .child(&content)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .build();
        scroll.add_css_class("context-menu-scroll");
        let popover = gtk::Popover::builder()
            .child(&scroll)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .build();
        popover.add_css_class("column-popover");
        let current_label = gtk::Label::new(Some(current));
        current_label.set_xalign(0.0);
        current_label.set_hexpand(true);
        current_label.set_max_width_chars(24);
        current_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let button = gtk::MenuButton::builder()
            .child(&current_label)
            .always_show_arrow(true)
            .popover(&popover)
            .build();
        crate::ui::accessibility::set_description(&button, Some(current));
        button.add_css_class("form-control");
        button.add_css_class("chooser-dropdown");
        button.set_halign(gtk::Align::Start);

        let scroll_for_show = scroll.clone();
        let button_for_show = button.downgrade();
        popover.connect_show(move |popover| {
            let (Some(root), Some(button)) = (popover.root(), button_for_show.upgrade()) else {
                return;
            };
            let anchor_top = button
                .compute_point(&root, &gtk::graphene::Point::new(0.0, 0.0))
                .map_or(0, |point| point.y().round() as i32);
            let (position, max_content_height) = dropdown_placement(
                root.height(),
                anchor_top,
                anchor_top.saturating_add(button.height()),
            );
            popover.set_position(position);
            scroll_for_show.set_max_content_height(max_content_height);
        });

        let selected = Rc::new(Cell::new(selected));
        let changed = Rc::new(RefCell::new(None::<SelectionChanged>));
        let checks = Rc::new(RefCell::new(Vec::<gtk::Image>::new()));
        for (index, label) in labels.iter().enumerate() {
            let (option, check) = menu_option(label, index == selected.get());
            if let Some(label_widget) = option
                .child()
                .and_then(|row| row.first_child())
                .and_downcast::<gtk::Label>()
            {
                label_widget.set_max_width_chars(48);
                label_widget.set_ellipsize(gtk::pango::EllipsizeMode::End);
            }
            checks.borrow_mut().push(check);
            let selected = selected.clone();
            let changed = changed.clone();
            let checks = checks.clone();
            let button = button.downgrade();
            let current_label = current_label.downgrade();
            let popover = popover.downgrade();
            let label = (*label).to_owned();
            option.connect_clicked(move |_| {
                selected.set(index);
                if let Some(current_label) = current_label.upgrade() {
                    current_label.set_label(&label);
                }
                if let Some(button) = button.upgrade() {
                    crate::ui::accessibility::set_description(&button, Some(&label));
                }
                for (check_index, check) in checks.borrow().iter().enumerate() {
                    check.set_visible(check_index == index);
                }
                if let Some(popover) = popover.upgrade() {
                    popover.popdown();
                }
                if let Some(changed) = changed.borrow().as_ref() {
                    changed(index);
                }
            });
            content.append(&option);
        }

        Self {
            button,
            popover,
            selected,
            changed,
        }
    }

    pub(super) fn selected(&self) -> usize {
        self.selected.get()
    }

    pub(super) fn connect_selected(&self, callback: impl Fn(usize) + 'static) {
        self.changed.replace(Some(Box::new(callback)));
    }

    pub(super) fn dismiss(&self) -> bool {
        if !self.popover.is_mapped() {
            return false;
        }
        self.popover.popdown();
        true
    }
}

impl ChoiceControl {
    pub(super) fn value(&self) -> (String, String) {
        match self {
            Self::Boolean { id, check } => (id.clone(), check.is_active().to_string()),
            Self::Select {
                id,
                values,
                dropdown,
            } => (
                id.clone(),
                values.get(dropdown.selected()).cloned().unwrap_or_default(),
            ),
        }
    }

    pub(super) fn dismiss_dropdown(&self) -> bool {
        match self {
            Self::Boolean { .. } => false,
            Self::Select { dropdown, .. } => dropdown.dismiss(),
        }
    }
}

pub(super) fn portal_filters(
    filters: &[FileFilter],
    current: Option<&FileFilter>,
) -> (Vec<PortalFilter>, Option<usize>) {
    let (filters, selected) = normalize_portal_filters(filters, current);
    (
        filters
            .into_iter()
            .map(|portal| {
                let native = gtk::FileFilter::new();
                native.set_name(Some(portal.label()));
                for pattern in portal.pattern_filters() {
                    native.add_pattern(pattern);
                }
                for mime in portal.mimetype_filters() {
                    native.add_mime_type(mime);
                }
                PortalFilter { portal, native }
            })
            .collect(),
        selected,
    )
}

pub(super) fn normalize_portal_filters(
    filters: &[FileFilter],
    current: Option<&FileFilter>,
) -> (Vec<FileFilter>, Option<usize>) {
    let mut filters = filters.to_vec();
    if let Some(current) = current
        && !filters.contains(current)
    {
        filters.push(current.clone());
    }
    let selected = current
        .and_then(|current| filters.iter().position(|filter| filter == current))
        .or_else(|| (!filters.is_empty()).then_some(0));
    (filters, selected)
}

pub(super) fn chooser_options() -> gtk::FlowBox {
    let options = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(false)
        .min_children_per_line(1)
        .max_children_per_line(32)
        .column_spacing(16)
        .row_spacing(6)
        .halign(gtk::Align::Start)
        .focusable(false)
        .build();
    options.add_css_class("chooser-options");
    options
}

pub(super) fn append_option(parent: &gtk::FlowBox, widget: &impl IsA<gtk::Widget>) {
    let child = gtk::FlowBoxChild::builder()
        .child(widget)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Center)
        .focusable(false)
        .build();
    parent.append(&child);
}

pub(super) fn build_choices(choices: &[Choice], parent: &gtk::FlowBox) -> Vec<ChoiceControl> {
    choices
        .iter()
        .map(|choice| {
            let pairs = choice.pairs();
            if pairs.is_empty() {
                let check = form_check_button(choice.label());
                check.set_active(choice.initial_selection() == "true");
                append_option(parent, &check);
                ChoiceControl::Boolean {
                    id: choice.id().to_owned(),
                    check,
                }
            } else {
                let labels = pairs.iter().map(|(_, label)| *label).collect::<Vec<_>>();
                let values = pairs
                    .iter()
                    .map(|(value, _)| (*value).to_owned())
                    .collect::<Vec<_>>();
                let selected = values
                    .iter()
                    .position(|value| value == choice.initial_selection())
                    .unwrap_or(0);
                let dropdown = ChooserDropdown::new(&labels, selected);
                let row = labeled_row(choice.label(), Some(dropdown.button.upcast_ref()));
                append_option(parent, &row);
                ChoiceControl::Select {
                    id: choice.id().to_owned(),
                    values,
                    dropdown,
                }
            }
        })
        .collect()
}

pub(super) fn labeled_row(label: &str, child: Option<&gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let label = form_label(label);
    label.set_max_width_chars(16);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    crate::ui::accessibility::set_description(&label, Some(&label.text()));
    row.append(&label);
    if let Some(child) = child {
        row.append(child);
    }
    row
}
