// SPDX-License-Identifier: GPL-3.0-or-later

use gtk::{gdk, glib, prelude::*};

#[cfg(test)]
mod tests;

pub(super) fn navigation_key(
    key: gdk::Key,
    modifiers: gdk::ModifierType,
    type_to_search: bool,
    focused: Option<&gtk::Widget>,
) -> gdk::Key {
    if type_to_search
        || modifiers.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK
                | gdk::ModifierType::SHIFT_MASK,
        )
        || focused.is_some_and(|widget| editable(widget) || in_popover(widget))
    {
        return key;
    }
    match key {
        gdk::Key::h => gdk::Key::Left,
        gdk::Key::j => gdk::Key::Down,
        gdk::Key::k => gdk::Key::Up,
        gdk::Key::l => gdk::Key::Right,
        _ => key,
    }
}

pub(super) fn activate_native_arrow(scope: &impl IsA<gtk::Widget>, key: gdk::Key) -> bool {
    let Some(focused) = scope.root().and_then(|root| root.focus()) else {
        return false;
    };
    let Some((collection, _)) = super::scrolling::focused_collection(&focused) else {
        return false;
    };
    // Invoke GTK's own binding: returning Proceed would deliver the original letter.
    let controllers = collection.observe_controllers();
    for index in 0..controllers.n_items() {
        let Some(controller) = controllers
            .item(index)
            .and_downcast::<gtk::ShortcutController>()
        else {
            continue;
        };
        for index in 0..controller.n_items() {
            let Some(shortcut) = controller.item(index).and_downcast::<gtk::Shortcut>() else {
                continue;
            };
            let Some(trigger) = shortcut.trigger().and_downcast::<gtk::KeyvalTrigger>() else {
                continue;
            };
            if trigger.keyval() == key
                && trigger.modifiers().is_empty()
                && let Some(action) = shortcut.action()
            {
                return action.activate(
                    gtk::ShortcutActionFlags::EXCLUSIVE,
                    &collection,
                    shortcut.arguments().as_ref(),
                );
            }
        }
    }
    false
}

pub(super) fn arrow_direction(key: gdk::Key) -> Option<gtk::DirectionType> {
    match key {
        gdk::Key::Left => Some(gtk::DirectionType::Left),
        gdk::Key::Right => Some(gtk::DirectionType::Right),
        gdk::Key::Up => Some(gtk::DirectionType::Up),
        gdk::Key::Down => Some(gtk::DirectionType::Down),
        _ => None,
    }
}

pub(super) fn editable(widget: &gtk::Widget) -> bool {
    widget.is::<gtk::Editable>()
        || widget.is::<gtk::TextView>()
        || widget.ancestor(gtk::Entry::static_type()).is_some()
        || widget.ancestor(gtk::TextView::static_type()).is_some()
}

pub(super) fn in_popover(widget: &gtk::Widget) -> bool {
    widget.is::<gtk::Popover>() || widget.ancestor(gtk::Popover::static_type()).is_some()
}

fn controls(scope: &gtk::Widget, result: &mut Vec<gtk::Widget>) {
    if !scope.is_mapped() || !scope.is_sensitive() {
        return;
    }
    if scope.is::<gtk::Popover>()
        || scope.is::<gtk::ListView>()
        || scope.is::<gtk::GridView>()
        || scope.is::<gtk::TextView>()
    {
        return;
    }
    if scope.is::<gtk::Button>()
        || scope.is::<gtk::MenuButton>()
        || scope.is::<gtk::CheckButton>()
        || scope.is::<gtk::Switch>()
        || scope.is::<gtk::Entry>()
    {
        result.push(scope.clone());
        return;
    }
    let mut child = scope.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        controls(&widget, result);
    }
}

fn focused_control(scope: &gtk::Widget) -> Option<(Vec<gtk::Widget>, usize)> {
    let focused = scope.root()?.focus()?;
    if editable(&focused) || in_popover(&focused) {
        return None;
    }
    let mut candidates = Vec::new();
    controls(scope, &mut candidates);
    let index = candidates
        .iter()
        .position(|candidate| *candidate == focused || focused.is_ancestor(candidate))?;
    Some((candidates, index))
}

pub(super) fn activate(scope: &gtk::Widget) -> bool {
    let Some((controls, current)) = focused_control(scope) else {
        return false;
    };
    let widget = &controls[current];
    if let Some(button) = widget.downcast_ref::<gtk::MenuButton>() {
        button.popup();
    } else if let Some(button) = widget.downcast_ref::<gtk::Button>() {
        button.emit_clicked();
    } else if let Some(check) = widget.downcast_ref::<gtk::CheckButton>() {
        check.set_active(!check.is_active());
    } else if let Some(switch) = widget.downcast_ref::<gtk::Switch>() {
        switch.set_active(!switch.is_active());
    } else {
        return false;
    }
    true
}

pub(super) fn move_focus(scope: &gtk::Widget, direction: gtk::DirectionType) -> bool {
    let Some((controls, current)) = focused_control(scope) else {
        return false;
    };
    if direction == gtk::DirectionType::Down
        && let Some(button) = controls[current].downcast_ref::<gtk::MenuButton>()
    {
        button.popup();
        return true;
    }
    let bounds = controls
        .iter()
        .map(|widget| widget.compute_bounds(scope))
        .collect::<Vec<_>>();
    let Some(origin) = bounds[current].as_ref() else {
        return false;
    };
    let target = bounds
        .iter()
        .enumerate()
        .filter_map(|(index, bounds)| {
            if index == current {
                return None;
            }
            directional_distance(origin, bounds.as_ref()?, direction).map(|score| (index, score))
        })
        .min_by(|(_, a), (_, b)| a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    target.is_some_and(|(index, _)| {
        let moved = controls[index].grab_focus()
            || controls[index].child_focus(gtk::DirectionType::TabForward);
        if moved && let Some(window) = scope.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(true);
        }
        moved
    })
}

fn directional_distance(
    origin: &gtk::graphene::Rect,
    target: &gtk::graphene::Rect,
    direction: gtk::DirectionType,
) -> Option<(bool, f32)> {
    let horizontal = matches!(
        direction,
        gtk::DirectionType::Left | gtk::DirectionType::Right
    );
    let (along, across, origin_size, target_size) = if horizontal {
        (
            target.center().x() - origin.center().x(),
            target.center().y() - origin.center().y(),
            origin.height(),
            target.height(),
        )
    } else {
        (
            target.center().y() - origin.center().y(),
            target.center().x() - origin.center().x(),
            origin.width(),
            target.width(),
        )
    };
    let sign = if matches!(direction, gtk::DirectionType::Left | gtk::DirectionType::Up) {
        -1.0
    } else {
        1.0
    };
    let distance = along * sign;
    (distance > 1.0).then_some((
        across.abs() * 2.0 > origin_size + target_size,
        distance + across.abs() * 2.0,
    ))
}

pub(super) fn install(scope: &impl IsA<gtk::Widget>) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = scope.as_ref().downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if modifiers.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK
                | gdk::ModifierType::SHIFT_MASK,
        ) {
            return glib::Propagation::Proceed;
        }
        let Some(scope) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let handled = if let Some(direction) = arrow_direction(key) {
            move_focus(&scope, direction)
        } else if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) {
            activate(&scope)
        } else {
            false
        };
        if handled {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    scope.add_controller(keys);
}
