//! The gallery page for C8 PlacesSidebar: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c8-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/C8-places-sidebar.md`.
//!
//! The page draws a header with the sidebar toggle, the sidebar, and a stand-in file pane: a
//! focusable `c8-files` element and a `c8-log` line naming the last intent, which scripts read.
//! The demo plays Strata's part with fake data: it opens places, pins the folders dropped into
//! a gap (a path with no extension is a folder), reorders, and marks an ejected device
//! pending. Page keys stand in for navigation elsewhere: Ctrl+B steps the sidebar, Alt+Up
//! opens Home, and 1–3 open a pin.

use rune::{Cx, El, Key, KeyEvent, Tx, col, el, row, text};
use strata_components::contract::dnd::DropHover;
use strata_components::sidebar::{
    Device, DeviceGroup, DropSite, ItemId, LockState, Pin, Place, PlacesSidebar, Release,
    SidebarData, SidebarIntent, SidebarState, Step, Via, WidthToggle,
};
use strata_kit::icons::IconName;

use crate::{Gallery, Scene};

/// C8's demo state, held in `Gallery::c8`.
pub struct Demo {
    pub sidebar: SidebarState,
    pub places: Vec<Place>,
    pub pins: Vec<Pin>,
    pub devices: Vec<DeviceGroup>,
    pub current: Option<ItemId>,
    pub tenxer: bool,
    /// What the last intent did, shown in `c8-log`.
    pub log: String,
}

impl Default for Demo {
    fn default() -> Demo {
        Demo {
            sidebar: SidebarState::default(),
            places: places(),
            pins: pins(),
            devices: devices(),
            current: Some(ItemId::Place("home".into())),
            tenxer: false,
            log: String::new(),
        }
    }
}

fn places() -> Vec<Place> {
    vec![
        Place::folder("home", "Home", IconName::Home).keycap("h"),
        Place::trash("Trash", true).keycap("t"),
        Place::other("network", "Network", IconName::Network).keycap("n"),
        Place::other("recent", "Recent", IconName::Recent).keycap("r"),
        Place::folder("desktop", "Desktop", IconName::Desktop),
        Place::folder("documents", "Documents", IconName::Documents).keycap("o"),
        Place::folder("downloads", "Downloads", IconName::Downloads).keycap("d"),
        Place::folder("music", "Music", IconName::Music).keycap("m"),
        Place::folder("pictures", "Pictures", IconName::Pictures).keycap("p"),
        Place::folder("videos", "Videos", IconName::Videos).keycap("v"),
    ]
}

fn pins() -> Vec<Pin> {
    vec![
        Pin::new("projects", "Projects").keycap("1"),
        Pin::new("photos", "Photos 2026").keycap("2"),
        Pin::remote("nas", "nas.local").keycap("3"),
    ]
}

/// Two drives, then a network share.
fn devices() -> Vec<DeviceGroup> {
    vec![
        DeviceGroup::new(vec![
            Device::new("system", "System", IconName::Drive, "/"),
            Device::new(
                "data",
                "129 GB Volume",
                IconName::Drive,
                "/run/media/demo/data",
            )
            .release(Release::Unmount),
        ]),
        DeviceGroup::new(vec![
            Device::new(
                "sandisk",
                "SanDisk",
                IconName::Usb,
                "/run/media/demo/SanDisk",
            )
            .release(Release::Eject),
        ]),
        DeviceGroup::new(vec![Device::new(
            "share",
            "projects on nas.local",
            IconName::Network,
            "smb://nas.local/projects",
        )]),
    ]
}

/// Every kind of device row: a fixed disk, an unmount-only volume, a USB stick, a locked and an
/// unlocked encrypted volume, a share, and a pending release after every group.
fn all_devices() -> Vec<DeviceGroup> {
    vec![
        DeviceGroup::new(vec![
            Device::new("system", "System", IconName::Drive, "/"),
            Device::new(
                "data",
                "129 GB Volume",
                IconName::Drive,
                "/run/media/demo/data",
            )
            .release(Release::Unmount),
        ]),
        DeviceGroup::new(vec![
            Device::new(
                "sandisk",
                "SanDisk",
                IconName::Usb,
                "/run/media/demo/SanDisk",
            )
            .release(Release::Eject),
            Device::new("vault", "Vault", IconName::Usb, "/dev/sdb2")
                .mounted(false)
                .lock(LockState::Locked),
            Device::new("backup", "Backup", IconName::Usb, "/run/media/demo/Backup")
                .lock(LockState::Unlocked)
                .release(Release::Eject),
        ]),
        DeviceGroup::new(vec![Device::new(
            "share",
            "projects on nas.local",
            IconName::Network,
            "smb://nas.local/projects",
        )]),
        DeviceGroup::new(vec![
            Device::new(
                "camera",
                "Camera",
                IconName::Phone,
                "/run/media/demo/Camera",
            )
            .pending(true),
        ])
        .divided(false),
    ]
}

impl Demo {
    fn data(&self) -> SidebarData<'_> {
        SidebarData {
            places: &self.places,
            pins: &self.pins,
            devices: &self.devices,
            current: self.current.as_ref(),
        }
    }

    fn device_mut(&mut self, id: &str) -> Option<&mut Device> {
        self.devices
            .iter_mut()
            .flat_map(|g| g.devices.iter_mut())
            .find(|d| d.id == id)
    }

    /// Strata's part, faked.
    fn apply(&mut self, intent: SidebarIntent) {
        self.log = match intent {
            SidebarIntent::Open { item, via } => {
                let how = match via {
                    Via::Pointer => "pointer",
                    Via::Keyboard => "keyboard",
                };
                let log = format!("Open {} ({how})", item.id());
                self.current = Some(item);
                log
            }
            SidebarIntent::OpenInNewTab { item } => format!("New tab {}", item.id()),
            SidebarIntent::ContextMenu { item, .. } => {
                format!("Menu {}", item.as_ref().map_or("sidebar", |i| i.id()))
            }
            SidebarIntent::MovePlace { from, to } => {
                let p = self.places.remove(from);
                self.places.insert(to, p);
                format!("Move place {from} to {to}")
            }
            SidebarIntent::MovePin { id, from, to } => {
                let p = self.pins.remove(from);
                self.pins.insert(to, p);
                format!("Move pin {id} to {to}")
            }
            SidebarIntent::PinDrop { gap, dropped } => {
                let paths = dropped.data.file_paths();
                // The fake check: a folder has no extension.
                if paths.is_empty() || paths.iter().any(|p| p.extension().is_some()) {
                    "Pin refused".into()
                } else {
                    let mut at = gap.index.min(self.pins.len());
                    let mut added = Vec::new();
                    for path in paths {
                        let name = path
                            .file_name()
                            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        let id = name.to_lowercase().replace(' ', "-");
                        if self.pins.iter().any(|p| p.id == id) {
                            continue;
                        }
                        self.pins.insert(at, Pin::new(id, name.clone()));
                        at += 1;
                        added.push(name);
                    }
                    format!("Pinned {} at {}", added.join(", "), gap.index)
                }
            }
            SidebarIntent::DropInto { item, dropped } => {
                format!(
                    "Drop {} into {}",
                    dropped.data.file_paths().len(),
                    item.id()
                )
            }
            SidebarIntent::Eject(id) => {
                if let Some(d) = self.device_mut(&id) {
                    d.pending = true;
                }
                format!("Eject {id}")
            }
            SidebarIntent::Unlock(id) => {
                if let Some(d) = self.device_mut(&id) {
                    d.lock = Some(LockState::Unlocked);
                }
                format!("Unlock {id}")
            }
            SidebarIntent::Lock(id) => {
                if let Some(d) = self.device_mut(&id) {
                    d.lock = Some(LockState::Locked);
                }
                format!("Lock {id}")
            }
            SidebarIntent::SectionToggled { section, collapsed } => {
                format!(
                    "{} {}",
                    section.title(),
                    if collapsed { "collapsed" } else { "expanded" }
                )
            }
            SidebarIntent::Stepped(step) => format!("Step {step:?}"),
            SidebarIntent::LeaveToFiles => "Back to the files".into(),
            // Hover reports change nothing here, so the log keeps the last action.
            SidebarIntent::PinHover { .. } | SidebarIntent::DropHover { .. } => return,
            _ => return,
        };
    }
}

fn sidebar_state(g: &mut Gallery) -> &mut SidebarState {
    &mut g.c8.sidebar
}

fn scene(g: &mut Gallery, dark: bool) {
    g.open("c8", dark);
    g.c8 = Demo {
        sidebar: SidebarState::new(&g.look),
        ..Demo::default()
    };
}

fn full(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    // A pin with a saved colour, as folder customization gives it.
    let tint = g.look.kit.c.warning.icon;
    g.c8.pins[1] = g.c8.pins[1].clone().tint(Some(tint));
}

fn rail(g: &mut Gallery, dark: bool) {
    full(g, dark);
    g.c8.sidebar.width = g.c8.sidebar.width.clone().with_step(Step::Rail);
}

fn hidden(g: &mut Gallery, dark: bool) {
    full(g, dark);
    g.c8.sidebar.width = g.c8.sidebar.width.clone().with_step(Step::Hidden);
}

fn with_devices(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    g.c8.devices = all_devices();
    g.c8.current = Some(ItemId::Device("sandisk".into()));
    // Room for every device row.
    g.c8.sidebar.pinned_collapsed = true;
}

fn gap_open(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    g.c8.sidebar
        .set_drop_hover(DropSite::Gap(1), DropHover::Accepting);
}

fn collapsed(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    g.c8.current = Some(ItemId::Pin("photos".into()));
    g.c8.sidebar.pinned_collapsed = true;
    g.c8.sidebar.devices_collapsed = true;
}

/// Files dragged over a pin's row body: the row's drop highlight.
fn drop_row(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    let item = ItemId::Pin("projects".into());
    g.c8.sidebar
        .set_drop_hover(DropSite::Item(item), DropHover::Accepting);
}

fn no_pins(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    g.c8.pins.clear();
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c8", dark)`, then fills in `g.c8`.
pub const SCENES: &[Scene] = &[
    ("c8-full-light", |g| full(g, false)),
    ("c8-full-dark", |g| full(g, true)),
    ("c8-rail-light", |g| rail(g, false)),
    ("c8-rail-dark", |g| rail(g, true)),
    ("c8-devices-light", |g| with_devices(g, false)),
    ("c8-devices-dark", |g| with_devices(g, true)),
    ("c8-gap-open-light", |g| gap_open(g, false)),
    ("c8-gap-open-dark", |g| gap_open(g, true)),
    ("c8-collapsed-light", |g| collapsed(g, false)),
    ("c8-collapsed-dark", |g| collapsed(g, true)),
    ("c8-drop-row-light", |g| drop_row(g, false)),
    ("c8-drop-row-dark", |g| drop_row(g, true)),
    ("c8-no-pins-light", |g| no_pins(g, false)),
    ("c8-no-pins-dark", |g| no_pins(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    ("c8-hidden-light", |g| hidden(g, false)),
    ("c8-hidden-dark", |g| hidden(g, true)),
    ("c8-keycaps-light", |g| {
        scene(g, false);
        g.c8.sidebar.keycaps = true;
    }),
    ("c8-keycaps-dark", |g| {
        scene(g, true);
        g.c8.sidebar.keycaps = true;
    }),
    ("c8-no-pins-collapsed-light", |g| {
        no_pins(g, false);
        g.c8.sidebar.pinned_collapsed = true;
    }),
    ("c8-no-pins-collapsed-dark", |g| {
        no_pins(g, true);
        g.c8.sidebar.pinned_collapsed = true;
    }),
    ("c8-tenxer-light", |g| {
        full(g, false);
        g.c8.tenxer = true;
    }),
    ("c8-tenxer-dark", |g| {
        full(g, true);
        g.c8.tenxer = true;
    }),
    ("c8-tenxer-rail-light", |g| {
        rail(g, false);
        g.c8.tenxer = true;
    }),
    ("c8-tenxer-rail-dark", |g| {
        rail(g, true);
        g.c8.tenxer = true;
    }),
    ("c8-rail-collapsed-light", |g| {
        rail(g, false);
        g.c8.sidebar.pinned_collapsed = true;
    }),
    ("c8-rail-collapsed-dark", |g| {
        rail(g, true);
        g.c8.sidebar.pinned_collapsed = true;
    }),
    ("c8-rail-no-pins-light", |g| {
        no_pins(g, false);
        g.c8.sidebar.width = g.c8.sidebar.width.clone().with_step(Step::Rail);
    }),
    ("c8-rail-no-pins-dark", |g| {
        no_pins(g, true);
        g.c8.sidebar.width = g.c8.sidebar.width.clone().with_step(Step::Rail);
    }),
];

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    cx.reduced_motion(!look.animations);
    let k = &look.kit;
    let d = &g.c8;
    let toggle = WidthToggle::new(&d.sidebar, sidebar_state)
        .on_intent(|g: &mut Gallery, i, _| g.c8.apply(i))
        .build(look);
    let header = row()
        .key("c8-header")
        .w_full()
        .h(look.frame.bar)
        .px(look.space.xs)
        .center_y()
        .child(toggle);
    let sidebar = PlacesSidebar::new(&d.sidebar, sidebar_state, d.data())
        .reorder(
            |g: &mut Gallery| &mut g.c8.sidebar.places_reorder,
            |g: &mut Gallery| &mut g.c8.sidebar.pins_reorder,
        )
        .tenxer(d.tenxer)
        .on_intent(|g: &mut Gallery, i, _: &mut Tx| g.c8.apply(i))
        .build(look);
    let files = col()
        .key("c8-pane")
        .grow(1.0)
        .h_full()
        .fill(k.c.layer_01)
        .padding(look.space.lg)
        .gap(look.space.sm)
        .child(
            el().key("c8-files")
                .role("list")
                .label("Files")
                .focusable(true)
                .on_press(|_: &mut Gallery| {})
                .w_full()
                .h(look.row.list),
        )
        .child(
            text(if d.log.is_empty() {
                "—".to_owned()
            } else {
                d.log.clone()
            })
            .key("c8-log")
            .font(k.ty.small)
            .color(k.c.text_muted),
        );
    col().key("c8").w_full().h_full().child(header).child(
        row()
            .key("c8-body")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(sidebar)
            .child(files),
    )
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, _tx: &mut Tx) -> bool {
    let d = &mut g.c8;
    match &key.key {
        Key::Command('b') => {
            let step = d.sidebar.width.advance();
            d.apply(SidebarIntent::Stepped(step));
            true
        }
        Key::Up if key.mods.alt => {
            d.current = Some(ItemId::Place("home".into()));
            true
        }
        Key::Text(t) if matches!(t.as_str(), "1" | "2" | "3") => {
            let i: usize = t.parse().unwrap_or(1);
            if let Some(p) = d.pins.get(i - 1) {
                d.current = Some(p.item());
            }
            true
        }
        _ => false,
    }
}

/// Runs at the start of every frame while this page is open, with the clock in seconds. It
/// drains a reorder's move that no press reported (a drag released off its row), as a host
/// must. True when state changed.
pub fn poll(g: &mut Gallery, _t: f64, _tx: &mut Tx) -> bool {
    match g.c8.sidebar.take_reordered() {
        Some(moved) => {
            g.c8.apply(moved);
            true
        }
        None => false,
    }
}
