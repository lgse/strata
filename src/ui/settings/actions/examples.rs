// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*};
use std::rc::Rc;

use super::{ACTION_EXAMPLES, EditorForm, editor_scroll, script_buffer};
use crate::ui::{
    controls::modal_layout,
    modal::{ModalHost, dismiss_modal_layer, modal_layer},
};

pub(super) fn show(anchor: &gtk::Button, form: Rc<EditorForm>) {
    let Some(host) = ModalHost::blurred_for(anchor) else {
        return;
    };
    let replacing = form.replaces_python_draft();
    let layout = modal_layout(
        crate::assets::icons::FILE_CODE,
        "Script examples",
        "Bundled Python recipes to review and customize",
        if replacing {
            "Replace script"
        } else {
            "Use example"
        },
    );
    layout.content.add_css_class("settings-action-examples");
    let names: Vec<_> = ACTION_EXAMPLES.iter().map(|example| example.name).collect();
    let choices = gtk::DropDown::from_strings(&names);
    choices.add_css_class("form-control");
    super::label_control(&choices, "Example script", "Choose a bundled Python recipe");
    layout.body.append(&choices);
    let description = note("");
    let requirements = note("");
    let inputs = note("");
    description.remove_css_class("settings-option-description");
    description.add_css_class("settings-option-title");
    for label in [&description, &requirements, &inputs] {
        layout.body.append(label);
    }
    let buffer = script_buffer("python3", "");
    let view = sourceview5::View::builder()
        .buffer(&buffer)
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .show_line_numbers(true)
        .left_margin(12)
        .right_margin(12)
        .top_margin(10)
        .bottom_margin(10)
        .build();
    super::label_control(
        &view,
        "Example code",
        "Read-only preview of the selected script",
    );
    let scroll = editor_scroll(&view);
    scroll.set_min_content_height(250);
    layout.body.append(&scroll);
    let warning = note(if replacing {
        "Replaces your Python draft (code can be undone in the editor). Applies the recipe's run mode and file filters; keeps your name, id, and other settings. Nothing runs or saves yet."
    } else {
        "Uses Python and applies the recipe's run mode and file filters. Keeps any name you entered. Nothing runs or saves yet."
    });
    layout.body.append(&warning);
    let update = move |index: u32| {
        let Some(example) = ACTION_EXAMPLES.get(index as usize) else {
            return;
        };
        description.set_text(example.description);
        requirements.set_text(&format!("Requires: {}", example.requirements));
        inputs.set_text(example.inputs);
        buffer.set_text(&example.script());
    };
    update(0);
    choices.connect_selected_notify(move |choices| update(choices.selected()));

    let layer = modal_layer(
        &layout.content,
        &host.overlay,
        host.blurred_root.clone(),
        Some(Rc::new(|| true)),
    );
    let dismiss: Rc<dyn Fn()> = Rc::new({
        let weak_layer = layer.downgrade();
        let overlay = host.overlay.clone();
        let root = host.blurred_root.clone();
        let anchor = anchor.downgrade();
        move || {
            if let Some(layer) = weak_layer.upgrade() {
                dismiss_modal_layer(&layer, &overlay, root.as_ref());
            }
            if let Some(anchor) = anchor.upgrade() {
                anchor.grab_focus();
            }
        }
    });
    for button in [&layout.cancel, &layout.close] {
        let dismiss = dismiss.clone();
        button.connect_clicked(move |_| dismiss());
    }
    let escape = gtk::EventControllerKey::new();
    let cancel = dismiss.clone();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            cancel();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(escape);
    layout.confirm.connect_clicked(move |_| {
        if let Some(example) = ACTION_EXAMPLES.get(choices.selected() as usize) {
            dismiss();
            form.apply_example(example);
        }
    });
    host.overlay.add_overlay(&layer);
    layout.cancel.grab_focus();
}

fn note(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_max_width_chars(84);
    label.add_css_class("settings-option-description");
    label
}
