// SPDX-License-Identifier: MIT

//! The bar: Strata's browser header, its action buttons and its window buttons.

use std::rc::Rc;

use rune::{El, Tx, row};
use rune_kit::{ControlSize, IconButton, IconButtonVariant};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::chrome::{HeaderLabels, Shown, WindowAction, WindowButtons, WindowChrome};

pub struct Header<'a> {
    key: String,
    look: &'a Look,
    leading: Vec<El>,
    center: Option<El>,
    actions: Vec<El>,
    window: Vec<El>,
}

impl<'a> Header<'a> {
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

    pub fn center(mut self, el: El) -> Self {
        self.center = Some(el);
        self
    }

    pub fn action(mut self, el: El) -> Self {
        self.actions.push(el);
        self
    }

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
        let accent_kit = look.accent_icon_kit();
        let mut button = |shown: bool, suffix: &str, icon: IconName, label: &str, act| {
            if !shown {
                return;
            }
            let on_action = on_action.clone();
            self.window.push(
                action(look, icon, label)
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| on_action(s, act, tx))
                    .build(&accent_kit)
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

    pub fn build(self) -> El {
        let look = self.look;
        let k = &look.kit;
        let middle = match self.center {
            Some(center) => row().grow(1.0).min_w(0.0).items_center().child(center),
            None => row().grow(1.0).min_w(0.0),
        };
        row()
            .key(self.key)
            .w_full()
            .h(look.frame.bar + 2.0 * look.frame.inset)
            .pl(look.space.xs)
            .pr(look.space.sm)
            .gap(look.space.xs)
            .items_center()
            .fill(k.c.bg_deep)
            .border_sides(
                [0.0, 0.0, 1.0, 0.0],
                look.transparency.frosted_border(k.c.border_base),
            )
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

pub fn action(look: &Look, icon: IconName, label: &str) -> IconButton {
    IconButton::new(look.icon(icon))
        .variant(IconButtonVariant::Ghost)
        .control_size(control_size(look.frame.control))
        .help(label.to_owned())
}

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
