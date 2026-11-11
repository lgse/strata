//! The bar: Strata's browser header, its action buttons and its window buttons.

use std::rc::Rc;

use rune::{El, Tx, row};
use rune_kit::{ControlSize, IconButton, IconButtonVariant};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::chrome::{HeaderLabels, Shown, WindowAction, WindowButtons, WindowChrome};

/// Strata's window header: the sidebar toggle, the location bar, the actions, then the window
/// buttons, left to right. A press on any part without press handlers of its own moves the
/// window, and a double press maximises or restores it (Rune's `.window_drag()`).
///
/// ```ignore
/// let bar = Header::new("header", look)
///     .leading(sidebar_toggle)
///     .center(location_bar)
///     .action(header::action(look, IconName::Plus, "New tab").on_press_tx(new_tab).build(&look.kit).key("new-tab"))
///     .window_buttons(prefs, chrome, &labels, |s: &mut State, action, tx| route(s, action, tx))
///     .build();
/// ```
///
/// The header holds no state. Its slots are the host's elements; the window buttons come from
/// data and report a [`WindowAction`], which the host routes to its platform layer.
pub struct Header<'a> {
    key: String,
    look: &'a Look,
    leading: Vec<El>,
    center: Option<El>,
    actions: Vec<El>,
    window: Vec<El>,
}

impl<'a> Header<'a> {
    /// An empty header keyed `key`; its window buttons are keyed `{key}-minimize`,
    /// `{key}-maximize` and `{key}-close`.
    pub fn new(key: impl Into<String>, look: &'a Look) -> Header<'a> {
        Header {
            key: key.into(),
            look,
            leading: Vec::new(),
            center: None,
            actions: Vec::new(),
            window: Vec::new(),
        }
    }

    /// An element before the location bar: Strata's sidebar toggle.
    pub fn leading(mut self, el: El) -> Self {
        self.leading.push(el);
        self
    }

    /// The flexible middle: Strata's location bar. Without it the middle is empty, and moves
    /// the window like the rest of the bar.
    pub fn center(mut self, el: El) -> Self {
        self.center = Some(el);
        self
    }

    /// An action after the location bar, in order: New tab, Search, Appearance, Settings.
    /// [`action`] builds one with the header's look.
    pub fn action(mut self, el: El) -> Self {
        self.actions.push(el);
        self
    }

    /// The window buttons: those `buttons` turns on, less any `chrome` says can't work
    /// ([`Shown`]). Maximize shows Restore on a maximised window, and a name that fits both
    /// while the state is unknown. Each press calls `on_action`. Calling it again replaces the
    /// buttons, so their keys stay unique.
    pub fn window_buttons<S: 'static>(
        mut self,
        buttons: WindowButtons,
        chrome: &WindowChrome,
        labels: &HeaderLabels,
        on_action: impl Fn(&mut S, WindowAction, &mut Tx) + 'static,
    ) -> Self {
        let shown = Shown::new(buttons, chrome);
        let on_action = Rc::new(on_action);
        self.window.clear();
        let look = self.look;
        let mut button = |shown: bool, suffix: &str, icon: IconName, label: &str, act| {
            if !shown {
                return;
            }
            let on_action = on_action.clone();
            self.window.push(
                action(look, icon, label)
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| on_action(s, act, tx))
                    .build(&look.kit)
                    .key(format!("{}-{suffix}", self.key)),
            );
        };
        let (max_icon, max_label) = labels.maximize_face(chrome.maximized);
        button(
            shown.minimize,
            "minimize",
            IconName::Minimize,
            &labels.minimize,
            WindowAction::Minimize,
        );
        button(
            shown.maximize,
            "maximize",
            max_icon,
            max_label,
            WindowAction::ToggleMaximize,
        );
        button(
            shown.close,
            "close",
            IconName::Close,
            &labels.close,
            WindowAction::Close,
        );
        self
    }

    /// The bar, keyed with the header's key: the leading elements, the flexible middle, then
    /// the actions and the window buttons, on the surface with a hairline underneath. It is a
    /// `.window_drag()` region; give it no press handlers of its own.
    pub fn build(self) -> El {
        let look = self.look;
        let k = &look.kit;
        let middle = match self.center {
            Some(center) => row().grow(1.0).min_w(0.0).items_center().child(center),
            None => row().grow(1.0).min_w(0.0),
        };
        // Strata's `headerbar.browser-header`: the surface, a bar's height plus the Frame
        // density inset above and below, a hairline at the border colour's 60 % underneath, and
        // the actions spaced by the inset. Strata puts the sidebar toggle 5 px in and the
        // actions 6 px from the end; the nearest spacing tokens are `space.xs` (4) and
        // `space.sm` (6).
        row()
            .key(self.key)
            .w_full()
            .h(look.frame.bar + 2.0 * look.frame.inset)
            .pl(look.space.xs)
            .pr(look.space.sm)
            .gap(look.space.xs)
            .items_center()
            .fill(k.c.bg_deep)
            .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_base)
            .window_drag()
            .children(self.leading)
            .child(middle)
            .child(
                row()
                    .gap(look.frame.inset)
                    .items_center()
                    .children(self.actions)
                    .children(self.window),
            )
    }
}

/// A header action in Strata's `header-action` look: a ghost icon button at the bar's control
/// size, in the text colour, named and tooltipped by `label`. Add its handler and key:
/// `action(look, IconName::Search, "Search").on_press_tx(..).build(&look.kit).key("search")`.
pub fn action(look: &Look, icon: IconName, label: &str) -> IconButton {
    IconButton::new(look.icon(icon))
        .variant(IconButtonVariant::Ghost)
        .control_size(control_size(look.frame.control))
        .help(label.to_owned())
}

/// The kit's button size nearest the look's bar control size (`frame.control`): rune-kit's
/// icon buttons come in 16, 20, 24, 28 and 36 point squares. The same function as C9's
/// (`preview_drawer/header.rs`), which is private to C9's module; sharing it means moving it to
/// a module both read, which is C9's file to change (a follow-up, not X1's).
fn control_size(points: f32) -> ControlSize {
    [
        (16.0, ControlSize::Mini),
        (20.0, ControlSize::Small),
        (24.0, ControlSize::Regular),
        (28.0, ControlSize::Large),
        (36.0, ControlSize::ExtraLarge),
    ]
    .into_iter()
    .min_by(|a, b| (a.0 - points).abs().total_cmp(&(b.0 - points).abs()))
    .map_or(ControlSize::Regular, |(_, size)| size)
}
