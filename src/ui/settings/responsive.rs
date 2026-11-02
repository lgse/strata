// SPDX-License-Identifier: MIT

//! The dialog's responsive shell: it sizes the panel to the window and reflows
//! navigation, rows, and choice matrices as the width changes.

use std::cell::{Cell, RefCell};

use gtk::{glib, prelude::*, subclass::prelude::*};

use super::{super::preferences::PreferenceManager, search};

pub(super) struct ResponsiveContent {
    pub(super) flows: Vec<(gtk::FlowBox, u32)>,
    pub(super) actions: Vec<(gtk::Box, gtk::Button)>,
    pub(super) setting_rows: Vec<gtk::Box>,
    pub(super) activation_rows: Vec<ResponsiveActivationRow>,
}

pub struct ResponsiveActivationRow {
    pub(super) row: gtk::Box,
    pub(super) options: Vec<gtk::Box>,
}

const DIALOG_WIDTH: i32 = 1400;
const DIALOG_HEIGHT: i32 = 1024;
const DIALOG_MARGIN: i32 = 24;
const COMPACT_NAVIGATION_BREAKPOINT: i32 = 900;
// Reflow the content before collapsing navigation: desktop toolbars need more room.
const COMPACT_CONTENT_BREAKPOINT: i32 = 1250;
const MIN_SIDE_BY_SIDE_ACTIVATION_WIDTH: i32 = 620;
const STACK_ACTIVATION_OPTIONS_BREAKPOINT: i32 = 350;
const STACK_TEXT_SIZE_BREAKPOINT: i32 = 600;
const STACK_EXCLUSION_INPUT_BREAKPOINT: i32 = 1000;

fn stacked_orientation(stacked: bool) -> gtk::Orientation {
    if stacked {
        gtk::Orientation::Vertical
    } else {
        gtk::Orientation::Horizontal
    }
}

fn set_css_class(widget: &impl IsA<gtk::Widget>, class: &str, present: bool) {
    if present {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

/// How the activation matrix lays out at the current width.
#[derive(Clone, Copy)]
struct ActivationLayout {
    compact: bool,
    stacked_options: bool,
}

mod responsive_bin {
    use super::*;

    #[derive(Default)]
    pub struct ResponsiveBin {
        pub compact_navigation: Cell<bool>,
        pub compact_content: Cell<bool>,
        pub typography_scale: Cell<f64>,
        // Matrix widths below which the translated choices were seen squeezed: side by side,
        // or with each label beside its control.
        pub activation_side_by_side_min: Cell<i32>,
        pub activation_inline_options_min: Cell<i32>,
        pub navigation: RefCell<Option<gtk::Box>>,
        pub navigation_heading: RefCell<Option<gtk::Label>>,
        pub navigation_labels: RefCell<Vec<gtk::Label>>,
        pub navigation_contents: RefCell<Vec<gtk::Box>>,
        pub responsive_flows: RefCell<Vec<(gtk::FlowBox, u32)>>,
        pub responsive_actions: RefCell<Vec<(gtk::Box, gtk::Button)>>,
        pub responsive_setting_rows: RefCell<Vec<gtk::Box>>,
        pub responsive_activation_rows: RefCell<Vec<ResponsiveActivationRow>>,
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
            let logical_width = f64::from(child_width) / self.typography_scale.get().max(0.1);
            let compact_content = logical_width < f64::from(COMPACT_CONTENT_BREAKPOINT);
            let compact = uses_compact_navigation(
                (f64::from(child_width) / self.typography_scale.get().max(0.1)) as i32,
            );
            if self.compact_navigation.replace(compact) != compact {
                self.apply_compact_navigation(compact);
            }
            if self.compact_content.replace(compact_content) != compact_content {
                self.apply_compact_content(compact_content);
            }
            let activation = self.activation_layout(child_width, compact);
            self.apply_activation_layout(activation);
            reflow_settings(
                &child,
                Reflow {
                    compact: compact_content,
                    activation_compact: activation.compact,
                    stack_text_size: logical_width < f64::from(STACK_TEXT_SIZE_BREAKPOINT),
                    stack_exclusion_input: logical_width
                        < f64::from(STACK_EXCLUSION_INPUT_BREAKPOINT),
                },
            );
            let x = ((width - child_width) / 2) as f32;
            let y = ((height - child_height) / 2) as f32;
            let transform = gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x, y));
            child.allocate(child_width, child_height, baseline, Some(transform));
            self.stack_squeezed_activation_choices(activation);
        }
    }

    impl ResponsiveBin {
        fn apply_compact_navigation(&self, compact: bool) {
            if let Some(navigation) = self.navigation.borrow().as_ref() {
                search::set_compact(navigation, compact);
                set_css_class(navigation, "compact", compact);
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
        }

        fn apply_compact_content(&self, compact: bool) {
            for (flow, expanded_columns) in self.responsive_flows.borrow().iter() {
                flow.set_max_children_per_line(if compact { 1 } else { *expanded_columns });
            }
            for (row, action) in self.responsive_actions.borrow().iter() {
                row.set_orientation(stacked_orientation(compact));
                action.set_halign(gtk::Align::Fill);
            }
            for row in self.responsive_setting_rows.borrow().iter() {
                row.set_orientation(stacked_orientation(compact));
                row.set_spacing(if compact { 8 } else { 16 });
            }
        }

        fn activation_layout(&self, child_width: i32, compact: bool) -> ActivationLayout {
            let available_activation_width = child_width - if compact { 100 } else { 330 };
            let scaled_activation_width = MIN_SIDE_BY_SIDE_ACTIVATION_WIDTH as f64
                + (self.typography_scale.get() - 1.0).max(0.0) * 320.0;
            let matrix_width = activation_matrix_width(&self.responsive_activation_rows.borrow());
            ActivationLayout {
                compact: f64::from(available_activation_width) < scaled_activation_width
                    || matrix_width < self.activation_side_by_side_min.get(),
                stacked_options: available_activation_width < STACK_ACTIVATION_OPTIONS_BREAKPOINT
                    || matrix_width < self.activation_inline_options_min.get(),
            }
        }

        fn apply_activation_layout(&self, layout: ActivationLayout) {
            for responsive_row in self.responsive_activation_rows.borrow().iter() {
                responsive_row
                    .row
                    .set_orientation(stacked_orientation(layout.compact));
                responsive_row
                    .row
                    .set_spacing(if layout.compact { 4 } else { 24 });
                for option in &responsive_row.options {
                    option.set_orientation(stacked_orientation(layout.stacked_options));
                    option.set_spacing(if layout.stacked_options { 2 } else { 6 });
                    option.set_halign(if layout.compact && !layout.stacked_options {
                        gtk::Align::End
                    } else {
                        gtk::Align::Fill
                    });
                }
                set_css_class(&responsive_row.row, "compact", layout.compact);
            }
        }

        /// Long translated choices can need more room than the fixed breakpoints allow; once
        /// a choice gets less than its natural width, stack the matrix further at this width.
        fn stack_squeezed_activation_choices(&self, layout: ActivationLayout) {
            if layout.compact && layout.stacked_options {
                return;
            }
            let rows = self.responsive_activation_rows.borrow();
            let squeezed = rows.iter().filter(|row| row.row.is_mapped()).any(|row| {
                row.options
                    .iter()
                    .filter_map(|option| option.last_child())
                    .any(|control| choice_label_wraps(&control))
            });
            if !squeezed {
                return;
            }
            let threshold = activation_matrix_width(&rows) + 1;
            if layout.compact {
                self.activation_inline_options_min.set(threshold);
            } else {
                self.activation_side_by_side_min.set(threshold);
            }
            let bin = self.obj().downgrade();
            glib::idle_add_local_once(move || {
                if let Some(bin) = bin.upgrade() {
                    bin.queue_allocate();
                }
            });
        }
    }
}

glib::wrapper! {
    pub struct ResponsiveBin(ObjectSubclass<responsive_bin::ResponsiveBin>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ResponsiveBin {
    pub(super) fn new(
        child: &impl IsA<gtk::Widget>,
        navigation: &gtk::Box,
        navigation_heading: &gtk::Label,
        responsive: ResponsiveContent,
    ) -> Self {
        let bin: Self = glib::Object::new();
        let imp = bin.imp();
        imp.navigation.replace(Some(navigation.clone()));
        imp.navigation_heading
            .replace(Some(navigation_heading.clone()));
        imp.responsive_flows.replace(responsive.flows);
        imp.responsive_actions.replace(responsive.actions);
        imp.responsive_setting_rows.replace(responsive.setting_rows);
        imp.responsive_activation_rows
            .replace(responsive.activation_rows);
        child.set_parent(&bin);
        PreferenceManager::shared().bind_interface_scale(&bin, |widget, scale| {
            let bin = widget
                .downcast_ref::<ResponsiveBin>()
                .expect("settings bin");
            bin.imp().typography_scale.set(scale);
            bin.imp().activation_side_by_side_min.set(0);
            bin.imp().activation_inline_options_min.set(0);
            bin.queue_allocate();
        });
        bin
    }

    pub(super) fn add_navigation(&self, label: gtk::Label, content: gtk::Box) {
        let imp = self.imp();
        imp.navigation_labels.borrow_mut().push(label);
        imp.navigation_contents.borrow_mut().push(content);
    }

    pub(super) fn add_flow(&self, flow: gtk::FlowBox, columns: u32) {
        flow.set_max_children_per_line(if self.imp().compact_content.get() {
            1
        } else {
            columns
        });
        self.imp()
            .responsive_flows
            .borrow_mut()
            .push((flow, columns));
    }

    pub(super) fn add_action(&self, row: gtk::Box, button: gtk::Button) {
        row.set_orientation(stacked_orientation(self.imp().compact_content.get()));
        button.set_halign(gtk::Align::Fill);
        self.imp()
            .responsive_actions
            .borrow_mut()
            .push((row, button));
    }
}

/// Whether a segmented control gives any choice less than its one-line width.
fn choice_label_wraps(control: &gtk::Widget) -> bool {
    let mut choice = control.first_child();
    while let Some(button) = choice {
        choice = button.next_sibling();
        if let Some(label) = button.downcast_ref::<gtk::Button>().and_then(|b| b.child())
            && label.width() < label.measure(gtk::Orientation::Horizontal, -1).1
        {
            return true;
        }
    }
    false
}

fn activation_matrix_width(rows: &[ResponsiveActivationRow]) -> i32 {
    rows.first()
        .and_then(|row| row.row.parent())
        .map_or(0, |matrix| matrix.width())
}

/// The widths crossed at the current size, applied to every widget in the dialog.
#[derive(Clone, Copy)]
struct Reflow {
    compact: bool,
    activation_compact: bool,
    stack_text_size: bool,
    stack_exclusion_input: bool,
}

const STACKING_ROW_CLASSES: [&str; 6] = [
    "settings-option",
    "settings-library-toolbar",
    "about-identity",
    "theme-library-footer",
    "settings-inline-description",
    "settings-update-summary",
];

fn reflow_settings(widget: &gtk::Widget, reflow: Reflow) {
    if widget.has_css_class("settings-dialog") {
        set_css_class(widget, "compact", reflow.compact);
    }
    if let Some(row) = widget.downcast_ref::<gtk::Box>()
        && STACKING_ROW_CLASSES
            .iter()
            .any(|class| row.has_css_class(class))
    {
        reflow_setting_row(row, reflow);
    }
    if widget.has_css_class("settings-integration-actions")
        && let Some(actions) = widget.downcast_ref::<gtk::Box>()
    {
        actions.set_orientation(stacked_orientation(reflow.compact));
    }
    if widget.has_css_class("theme-appearance-filter")
        || widget.has_css_class("settings-integration-actions")
    {
        reflow_button_group(widget, reflow.compact);
    }
    reflow_labels(widget, reflow);
    // Keep the exclusion placeholder readable instead of squeezing it beside translated buttons.
    if widget.has_css_class("settings-exclusions-input")
        && let Some(row) = widget.downcast_ref::<gtk::Box>()
    {
        set_orientation_if_changed(row, stacked_orientation(reflow.stack_exclusion_input));
    }
    if widget.has_css_class("activation-header") {
        widget.set_visible(!reflow.activation_compact);
    }
    if widget.has_css_class("activation-inline-label") {
        widget.set_visible(reflow.activation_compact);
    }
    let mut child = widget.first_child();
    while let Some(next) = child {
        child = next.next_sibling();
        reflow_settings(&next, reflow);
    }
}

fn reflow_setting_row(row: &gtk::Box, reflow: Reflow) {
    let switch_row = row
        .last_child()
        .is_some_and(|child| child.is::<gtk::Switch>());
    // Short numeric controls can stay beside wrapping copy after other rows stack.
    let stack_row = if row.has_css_class("settings-renderer-row")
        || row.has_css_class("settings-exclusions-row")
    {
        true
    } else if row.has_css_class("settings-text-size-row") {
        reflow.stack_text_size
    } else {
        reflow.compact
    };
    let orientation = stacked_orientation(stack_row && !switch_row);
    // Release notes and sidebar chips always stay below their row's copy.
    let pinned =
        row.has_css_class("settings-update-status") || row.has_css_class("settings-sidebar-places");
    if !pinned {
        set_orientation_if_changed(row, orientation);
    }
}

fn set_orientation_if_changed(row: &gtk::Box, orientation: gtk::Orientation) {
    if row.orientation() != orientation {
        row.set_orientation(orientation);
    }
}

fn reflow_button_group(widget: &gtk::Widget, compact: bool) {
    widget.set_halign(if compact {
        gtk::Align::Fill
    } else {
        gtk::Align::End
    });
    widget.set_hexpand(compact);
    let mut child = widget.first_child();
    while let Some(button) = child {
        child = button.next_sibling();
        button.set_hexpand(compact);
    }
}

fn reflow_labels(widget: &gtk::Widget, reflow: Reflow) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && (label.has_css_class("settings-nowrap") || label.has_css_class("menu-heading"))
    {
        label.set_wrap(reflow.compact && !label.has_css_class("settings-keycap"));
    }
    if let Some(button) = widget.downcast_ref::<gtk::Button>()
        && !widget.is::<gtk::ToggleButton>()
        && let Some(label) = button.child().and_downcast::<gtk::Label>()
    {
        label.set_wrap(reflow.compact);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    }
    // Beside the choices, keep long single-word view names whole; stacked, they have the full width.
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && label.has_css_class("click-activation-title")
    {
        let mode = if reflow.activation_compact {
            gtk::pango::WrapMode::WordChar
        } else {
            gtk::pango::WrapMode::Word
        };
        if label.wrap_mode() != mode {
            label.set_wrap_mode(mode);
        }
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
