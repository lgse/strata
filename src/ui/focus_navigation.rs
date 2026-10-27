// SPDX-License-Identifier: MIT

use gtk::{gdk, glib, prelude::*};

use super::keys::{
    self, Binding, KeyPress,
    bindings::{
        DOWN, ENTER, FOCUS_NEXT, FOCUS_PREVIOUS, LEFT, RIGHT, UP, VIM_DOWN, VIM_LEFT, VIM_RIGHT,
        VIM_UP,
    },
};

#[cfg(test)]
mod tests;

/// Shift and Super ride along with keys that move focus or the cursor.
pub(super) const FOCUS_MODIFIERS: gdk::ModifierType =
    gdk::ModifierType::SHIFT_MASK.union(gdk::ModifierType::SUPER_MASK);

const ARROWS: [(Binding, Binding); 4] = [
    (LEFT, VIM_LEFT),
    (DOWN, VIM_DOWN),
    (UP, VIM_UP),
    (RIGHT, VIM_RIGHT),
];

pub(super) fn spatial_arrow(press: &KeyPress) -> Option<Binding> {
    ARROWS
        .into_iter()
        .find(|(arrow, vim)| press.is_any(&[*arrow, *vim]))
        .map(|(arrow, _)| arrow)
}

/// Reads **h j k l** as arrows unless typing owns them.
pub(super) fn navigation_key(
    press: &KeyPress,
    type_to_search: bool,
    focused: Option<&gtk::Widget>,
) -> KeyPress {
    if type_to_search || focused.is_some_and(|widget| editable(widget) || in_popover(widget)) {
        return *press;
    }
    ARROWS
        .into_iter()
        .find(|(_, vim)| press.is(*vim))
        .map_or(*press, |(arrow, _)| press.read_as(arrow))
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

pub(super) fn arrow_direction(
    press: &KeyPress,
    tolerated: gdk::ModifierType,
) -> Option<gtk::DirectionType> {
    [
        (LEFT, gtk::DirectionType::Left),
        (RIGHT, gtk::DirectionType::Right),
        (UP, gtk::DirectionType::Up),
        (DOWN, gtk::DirectionType::Down),
    ]
    .into_iter()
    .find(|(arrow, _)| press.is(arrow.ignoring(tolerated)))
    .map(|(_, direction)| direction)
}

pub(super) fn vim_focus_direction(press: &KeyPress) -> Option<gtk::DirectionType> {
    [
        (VIM_LEFT, gtk::DirectionType::Left),
        (VIM_DOWN, gtk::DirectionType::Down),
        (VIM_UP, gtk::DirectionType::Up),
        (VIM_RIGHT, gtk::DirectionType::Right),
    ]
    .into_iter()
    .find_map(|(binding, direction)| press.is(binding).then_some(direction))
}

pub(super) fn plain_tab_direction(press: &KeyPress) -> Option<gtk::DirectionType> {
    if press.is(FOCUS_NEXT) {
        Some(gtk::DirectionType::TabForward)
    } else if press.is(FOCUS_PREVIOUS) {
        Some(gtk::DirectionType::TabBackward)
    } else {
        None
    }
}

/// Listing updates must not take focus from a modal or an inactive tab.
pub(super) fn browser_can_take_focus(widget: &impl IsA<gtk::Widget>) -> bool {
    let widget = widget.as_ref();
    // Startup/loading focus can precede mapping; only an inactive tab is excluded.
    if !widget.is_mapped() {
        let mut child = widget.clone();
        while let Some(parent) = child.parent() {
            if child.has_css_class("tab-context")
                && parent.downcast_ref::<gtk::Stack>().is_some_and(|stack| {
                    stack
                        .visible_child()
                        .is_some_and(|visible| visible != child)
                })
            {
                return false;
            }
            child = parent;
        }
    }
    widget
        .root()
        .and_downcast::<gtk::Window>()
        .is_none_or(|window| !super::window::foreign_modal_visible(&window, None))
}

pub(super) fn contains_widget(container: &gtk::Widget, focused: Option<&gtk::Widget>) -> bool {
    focused.is_some_and(|focused| focused == container || focused.is_ancestor(container))
}

/// Moves focus to the next (`TabForward`) or previous (`TabBackward`) focusable widget
/// outside `scope` in Tab order, wrapping at the ends of the window as GTK does. Never
/// enters `scope`, so a whole subtree acts as one Tab stop.
pub(super) fn focus_beyond(scope: &gtk::Widget, direction: gtk::DirectionType) -> bool {
    let backward = direction == gtk::DirectionType::TabBackward;
    let step = |widget: &gtk::Widget| {
        if backward {
            widget.prev_sibling()
        } else {
            widget.next_sibling()
        }
    };
    let focus = |candidate: &gtk::Widget| {
        candidate.is_mapped() && candidate.is_sensitive() && candidate.child_focus(direction)
    };
    let mut path = vec![scope.clone()];
    while let Some(parent) = path.last().and_then(gtk::Widget::parent) {
        path.push(parent);
    }
    for widget in &path {
        let mut sibling = step(widget);
        while let Some(candidate) = sibling {
            if focus(&candidate) {
                return true;
            }
            sibling = step(&candidate);
        }
    }
    for widget in path.iter().rev() {
        let Some(parent) = widget.parent() else {
            continue;
        };
        let mut sibling = if backward {
            parent.last_child()
        } else {
            parent.first_child()
        };
        while let Some(candidate) = sibling.filter(|candidate| candidate != widget) {
            if focus(&candidate) {
                return true;
            }
            sibling = step(&candidate);
        }
    }
    false
}

/// Whether `focused` was detached from `container` while the window's focus chain still
/// runs through it (GTK moves focus at the next paint).
pub(super) fn focus_removed_from(container: &gtk::Widget, focused: &gtk::Widget) -> bool {
    if focused.root().is_some() {
        return false;
    }
    let mut child = container.clone();
    while let Some(parent) = child.parent() {
        if parent.focus_child().as_ref() != Some(&child) {
            return false;
        }
        child = parent;
    }
    child.is::<gtk::Root>()
}

pub(super) fn editable(widget: &gtk::Widget) -> bool {
    widget.is::<gtk::Editable>()
        || widget.is::<gtk::TextView>()
        || widget.ancestor(gtk::Entry::static_type()).is_some()
        || widget.ancestor(gtk::TextView::static_type()).is_some()
}

/// Unlike `editable`, a read-only text view is a document, not a field.
pub(super) fn accepts_typing(widget: &gtk::Widget) -> bool {
    let view = widget.downcast_ref::<gtk::TextView>().cloned().or_else(|| {
        widget
            .ancestor(gtk::TextView::static_type())
            .and_downcast::<gtk::TextView>()
    });
    match view {
        Some(view) => view.is_editable(),
        None => editable(widget),
    }
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
    if let Some(tab) = scope.downcast_ref::<gtk::Notebook>().and_then(current_tab) {
        result.push(tab);
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

/// A notebook's current tab stands in for the notebook, which holds focus while a
/// tab does.
fn current_tab(notebook: &gtk::Notebook) -> Option<gtk::Widget> {
    if !notebook.shows_tabs() {
        return None;
    }
    let page = notebook.nth_page(notebook.current_page())?;
    notebook
        .tab_label(&page)?
        .parent()
        .filter(gtk::Widget::is_mapped)
}

fn tab_owner(widget: &gtk::Widget) -> Option<gtk::Notebook> {
    widget
        .ancestor(gtk::Notebook::static_type())
        .and_downcast::<gtk::Notebook>()
        .filter(|notebook| current_tab(notebook).as_ref() == Some(widget))
}

fn focused_control(scope: &gtk::Widget) -> Option<(Vec<gtk::Widget>, usize)> {
    let focused = scope.root()?.focus()?;
    if editable(&focused) || in_popover(&focused) {
        return None;
    }
    let focused = match focused.downcast_ref::<gtk::Notebook>() {
        Some(notebook) => current_tab(notebook)?,
        None => focused,
    };
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
    // GTK's own Left and Right move between tabs.
    if matches!(
        direction,
        gtk::DirectionType::Left | gtk::DirectionType::Right
    ) && tab_owner(&controls[current]).is_some()
    {
        return false;
    }
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
    let vertical = matches!(direction, gtk::DirectionType::Up | gtk::DirectionType::Down);
    let target = bounds
        .iter()
        .enumerate()
        .filter_map(|(index, bounds)| {
            if index == current {
                return None;
            }
            let mut bounds = *bounds.as_ref()?;
            // The tab strip runs across the whole notebook, so Up and Down treat it as
            // directly above or below the control they leave.
            if vertical && tab_owner(&controls[index]).is_some() {
                bounds = gtk::graphene::Rect::new(
                    origin.x(),
                    bounds.y(),
                    origin.width(),
                    bounds.height(),
                );
            }
            directional_distance(origin, &bounds, direction).map(|score| (index, score))
        })
        .min_by(|(_, a), (_, b)| a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    target.is_some_and(|(index, _)| {
        let target = &controls[index];
        let moved = match tab_owner(target) {
            Some(notebook) => notebook.grab_focus(),
            None => target.grab_focus() || target.child_focus(gtk::DirectionType::TabForward),
        };
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
    keys::on_press(&keys, move |_, press| {
        let Some(scope) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let handled = if let Some(direction) = arrow_direction(press, gdk::ModifierType::empty()) {
            move_focus(&scope, direction)
        } else if press.is(ENTER) {
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
