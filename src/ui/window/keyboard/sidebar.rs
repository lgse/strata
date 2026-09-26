// SPDX-License-Identifier: MIT

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    prelude::*,
};

/// Sidebar chords while 10xer mode is on. `None` means a modified command that
/// the rest of the keymap still owns, such as Ctrl+L.
pub(in crate::ui) enum SidebarChord {
    Move(i32),
    Activate,
    Leave,
    Swallow,
}

pub(in crate::ui) fn sidebar_chord(key: Key, modifiers: Modifiers) -> Option<SidebarChord> {
    if modifiers.intersects(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK) {
        return None;
    }
    if modifiers.contains(Modifiers::SHIFT_MASK) {
        return Some(SidebarChord::Swallow);
    }
    Some(match key {
        Key::j | Key::Down | Key::KP_Down => SidebarChord::Move(1),
        Key::k | Key::Up | Key::KP_Up => SidebarChord::Move(-1),
        Key::l | Key::Return | Key::KP_Enter | Key::space => SidebarChord::Activate,
        Key::h | Key::Left | Key::KP_Left | Key::BackSpace => SidebarChord::Leave,
        Key::Right | Key::KP_Right | Key::Delete => SidebarChord::Swallow,
        _ => return None,
    })
}

pub(in crate::ui) fn move_sidebar_focus(sidebar: &gtk::Widget, delta: i32) -> bool {
    let targets = focus_targets(sidebar);
    let Some(target) = targets.first() else {
        return false;
    };
    let focused = sidebar.root().and_then(|root| root.focus());
    let index = focused.as_ref().and_then(|focused| {
        targets
            .iter()
            .position(|target| focused == target || focused.is_ancestor(target))
    });
    let next = match index {
        Some(index) => {
            let next = index as i32 + delta;
            if next < 0 || next >= targets.len() as i32 {
                reveal_focus(sidebar);
                return true;
            }
            next as usize
        }
        None => {
            if delta < 0 {
                targets.len() - 1
            } else {
                0
            }
        }
    };
    let moved = targets.get(next).unwrap_or(target).grab_focus();
    if moved {
        reveal_focus(sidebar);
    }
    moved
}

pub(in crate::ui) fn activate_sidebar_focus(sidebar: &gtk::Widget) -> bool {
    let Some(focused) = sidebar.root().and_then(|root| root.focus()) else {
        return false;
    };
    let Some(button) = focused
        .downcast_ref::<gtk::Button>()
        .cloned()
        .or_else(|| focused.ancestor(gtk::Button::static_type()).and_downcast())
    else {
        return false;
    };
    if !button.is_ancestor(sidebar) {
        return false;
    }
    // GtkButton's activate signal is "clicked". Emit it directly so a key
    // handler can run the same handler a pointer click uses.
    button.emit_clicked();
    true
}

fn reveal_focus(sidebar: &gtk::Widget) {
    if let Some(window) = sidebar.root().and_downcast::<gtk::Window>() {
        window.set_focus_visible(true);
    }
}

fn focus_targets(sidebar: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut targets = Vec::new();
    collect_targets(sidebar, &mut targets);
    targets
}

fn collect_targets(widget: &gtk::Widget, targets: &mut Vec<gtk::Widget>) {
    if widget.is::<gtk::Popover>() {
        return;
    }
    if widget.is::<gtk::Button>()
        && widget.is_sensitive()
        && widget.is_visible()
        && widget.is_mapped()
    {
        targets.push(widget.clone());
        return;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        collect_targets(&widget, targets);
        child = widget.next_sibling();
    }
}
