//! One shell window's state and view: the header, an empty themed window with a readout of
//! what the platform layer said, and the mounted-volumes dialog. The gallery page draws it on
//! fake data; `examples/shell` draws it on the real platform shims, so both run the same view.

use rune::{Cx, El, Tx, col, row, text};
use strata_components::dialog::StrataDialog;
use strata_components::header::{
    self, Header, HeaderLabels, WindowAction, WindowButtons, WindowChrome,
};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

/// What the window's readout shows of the desktop's settings (strata-platform's `settings`,
/// as plain data).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Desktop {
    pub text_scale: f32,
    /// The desktop's double-click interval. Nothing in the shell uses it: the header's double
    /// press pairs within Rune's fixed 0.5 s (strata-platform draft U13, not filed).
    pub double_click_ms: u32,
    /// The desktop's animations switch: false turns the look's animations off.
    pub animations: bool,
}

impl Default for Desktop {
    /// strata-platform's defaults without a Settings portal.
    fn default() -> Desktop {
        Desktop {
            text_scale: 1.0,
            double_click_ms: 400,
            animations: true,
        }
    }
}

/// Where the overrides file stands, as the readout shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OverridesStatus {
    /// The file was there and read; false is no file, or one that couldn't be read.
    pub present: bool,
    /// Its rows with a problem, which don't apply.
    pub problems: usize,
    /// How many times it was read again since the window opened.
    pub reloads: u32,
}

/// One mounted volume, as the dialog lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    pub name: String,
    /// Where it is mounted, or what else the host says of it.
    pub detail: String,
}

/// The mounted-volumes listing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Volumes {
    #[default]
    Loading,
    Listed(Vec<Volume>),
    /// No GLib bridge, or the listing failed.
    Failed(String),
}

/// One shell window's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShellWindow {
    /// The launch argument the window shows, in place of the location bar (C6).
    pub location: String,
    pub buttons: WindowButtons,
    pub chrome: WindowChrome,
    pub labels: HeaderLabels,
    pub desktop: Desktop,
    pub overrides: OverridesStatus,
    pub volumes: Volumes,
    pub volumes_open: bool,
    /// The last thing a header button asked for, shown in the readout.
    pub last: Option<String>,
}

/// What the view calls in its host.
pub struct Hooks<S> {
    pub at: fn(&mut S) -> &mut ShellWindow,
    /// A window button: the host routes it to its platform layer.
    pub on_action: fn(&mut S, WindowAction, &mut Tx),
    /// The volumes dialog opened, or its Refresh was pressed: list the volumes again.
    pub on_volumes: fn(&mut S),
}

impl<S> Clone for Hooks<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Hooks<S> {}

/// The window: the header, wrapped by `wrap_header` (the host's window menu), over an empty
/// body with the readout, in the window's frame, wrapped by `wrap_window` (the host's resize
/// edges, under the overlay layer), with the volumes dialog on the overlay layer. Sets Rune's reduced motion from the look's animations switch for this
/// commit, and follows the window's focus into `chrome.active` when it is the view's root.
pub fn view<S: 'static>(
    w: &ShellWindow,
    look: &Look,
    cx: &mut Cx<S>,
    hooks: Hooks<S>,
    wrap_header: impl FnOnce(El) -> El,
    wrap_window: impl FnOnce(El) -> El,
) -> El {
    let reduced = !look.animations || cx.system_reduced_motion();
    cx.reduced_motion(reduced);
    let bar = wrap_header(bar(w, look, "x1", true, hooks));
    let body = col()
        .key("x1-body")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .fill(look.kit.c.bg_base)
        .padding(look.space.xl)
        .justify_end()
        .child(readout(w, look));
    let window = wrap_window(header::frame(
        "x1-frame",
        col().w_full().h_full().child(bar).child(body),
        look,
        &w.chrome,
    ));
    let mut layer = Vec::new();
    volumes_dialog(w, look, reduced, hooks, &mut layer);
    let at = hooks.at;
    // Rune calls this only on the view's root: in `examples/shell` it is; on the gallery page
    // the gallery's own root is, so there the scene sets `active` instead.
    rune_kit::root(window, layer).on_window_focus(move |s: &mut S, focused, _: &mut Tx| {
        at(s).chrome.active = focused;
    })
}

/// The window's header, its keys starting `{prefix}-`, with the demo's Mounted volumes action
/// when `volumes`. The sidebar toggle, the location bar and Strata's actions are stand-ins for
/// C8, C6, C7, C17 and Strata: a press says what it stands for in the readout.
pub fn bar<S: 'static>(
    w: &ShellWindow,
    look: &Look,
    prefix: &str,
    volumes: bool,
    hooks: Hooks<S>,
) -> El {
    let at = hooks.at;
    let k = &look.kit;
    let stand_in = |part: &str, icon: IconName, label: &'static str| {
        header::action(look, icon, label)
            .on_press(move |s: &mut S| at(s).last = Some(format!("{label} (stand-in)")))
            .build(k)
            .key(format!("{prefix}-{part}"))
    };
    let location = text(w.location.clone())
        .key(format!("{prefix}-location"))
        .font(k.ty.ui_medium)
        .color(k.c.text_base)
        .max_lines(1);
    let mut bar = Header::new(format!("{prefix}-header"), look)
        .leading(stand_in(
            "sidebar-toggle",
            IconName::Sidebar,
            "Toggle sidebar",
        ))
        .center(location)
        .action(stand_in("new-tab", IconName::Plus, "New tab"))
        .action(stand_in("search", IconName::Search, "Search"))
        .action(stand_in("appearance", IconName::Menu, "Appearance"))
        .action(stand_in("settings", IconName::Settings, "Settings"));
    if volumes {
        bar = bar.action(
            header::action(look, IconName::Drive, "Mounted volumes")
                .on_press(move |s: &mut S| {
                    at(s).volumes_open = true;
                    (hooks.on_volumes)(s);
                })
                .build(k)
                .key(format!("{prefix}-mounts")),
        );
    }
    bar.window_buttons(
        w.buttons,
        &w.chrome,
        &w.labels,
        move |s: &mut S, action, tx: &mut Tx| {
            at(s).last = Some(format!("{action:?}"));
            (hooks.on_action)(s, action, tx);
        },
    )
    .build()
}

/// The readout: one line per value, each its own label so scripts can read it.
fn readout(w: &ShellWindow, look: &Look) -> El {
    let k = &look.kit;
    let line = |key: &str, content: String| {
        text(content)
            .key(key.to_owned())
            .font(k.ty.small)
            .color(k.c.text_muted)
    };
    let d = &w.desktop;
    let o = &w.overrides;
    // The values after "·" are read from the look itself, so a script sees whether a change
    // reached it, not only that the host heard of one.
    let overrides = match (o.present, o.problems) {
        (false, _) => "Overrides: no file".to_owned(),
        (true, 0) => "Overrides: applied".to_owned(),
        (true, 1) => "Overrides: applied, 1 row with a problem".to_owned(),
        (true, n) => format!("Overrides: applied, {n} rows with a problem"),
    };
    let overrides = format!("{overrides} · bar inset {} pt", look.frame.inset);
    let reloads = match o.reloads {
        1 => "Overrides reloaded 1 time".to_owned(),
        n => format!("Overrides reloaded {n} times"),
    };
    col()
        .key("x1-readout")
        .gap(look.space.xxs)
        .child(line(
            "x1-text-scale",
            format!(
                "Text scale {} · interface text {} pt",
                d.text_scale, k.ty.ui.size
            ),
        ))
        .child(line(
            "x1-double-click",
            format!(
                "Desktop double-click {} ms (the header pairs within Rune's 0.5 s: U13)",
                d.double_click_ms
            ),
        ))
        .child(line(
            "x1-animations",
            format!("Animations {}", if look.animations { "on" } else { "off" }),
        ))
        .child(line("x1-overrides", overrides))
        .child(line("x1-reloads", reloads))
        .child(line(
            "x1-last",
            match &w.last {
                Some(last) => format!("Last header button: {last}"),
                None => "Last header button: none".to_owned(),
            },
        ))
}

fn volumes_dialog<S: 'static>(
    w: &ShellWindow,
    look: &Look,
    reduced: bool,
    hooks: Hooks<S>,
    layer: &mut Vec<El>,
) {
    let at = hooks.at;
    let k = &look.kit;
    let close = move |s: &mut S| at(s).volumes_open = false;
    let mut dialog = StrataDialog::new("x1-volumes", "Mounted volumes", close)
        .open(w.volumes_open)
        .reduced_motion(reduced)
        .icon(IconName::Drive)
        .subtitle("Listed by GIO on the GLib bridge")
        .secondary("Refresh", move |s: &mut S| (hooks.on_volumes)(s))
        .primary("Close", close);
    let body = match &w.volumes {
        Volumes::Loading => {
            dialog = dialog.loading(Some("Listing volumes…".into()));
            None
        }
        Volumes::Failed(why) => Some(dialog_text(look, "x1-volumes-error", why.clone())),
        Volumes::Listed(list) if list.is_empty() => Some(dialog_text(
            look,
            "x1-volumes-empty",
            "No mounted volumes".into(),
        )),
        Volumes::Listed(list) => Some(
            col()
                .key("x1-volumes-list")
                .role("list")
                .label("Mounted volumes")
                .gap(look.space.xs)
                .children(list.iter().enumerate().map(|(i, v)| {
                    row()
                        .key(format!("x1-volume-{i}"))
                        .role("listitem")
                        .label(v.name.clone())
                        .gap(look.space.md)
                        .items_center()
                        .child(
                            rune::icon(look.icon(IconName::Drive), look.icon_size.md)
                                .color(k.c.icon_muted)
                                .a11y_hidden(),
                        )
                        .child(
                            col()
                                .min_w(0.0)
                                .child(
                                    text(v.name.clone())
                                        .font(k.ty.ui)
                                        .color(k.c.text_base)
                                        .max_lines(1)
                                        .a11y_hidden(),
                                )
                                .child(
                                    text(v.detail.clone())
                                        .font(k.ty.small)
                                        .color(k.c.text_muted)
                                        .max_lines(1),
                                ),
                        )
                })),
        ),
    };
    if let Some(body) = body {
        dialog = dialog.child(body);
    }
    dialog.build(look, layer);
}

fn dialog_text(look: &Look, key: &str, content: String) -> El {
    text(content)
        .key(key.to_owned())
        .font(look.kit.ty.ui)
        .color(look.kit.c.text_muted)
}
