//! The gallery page for C7 BrowserTabs: its scenes, demo state, view and key and poll hooks. Scene
//! names start with `c7-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/C7-browser-tabs.md`.
//!
//! The page plays Strata's part with fake folders: the strip, a stand-in view per tab, and two
//! lines scripts read, `c7-log` (what the last intent did) and `c7-title` (the window title,
//! which the page also sets with `Tx::set_title`).
//!
//! - **Keys** stand in for Strata's key map: Ctrl+T, Ctrl+W, Ctrl+Shift+T, Ctrl+Tab and
//!   Ctrl+Shift+Tab, Ctrl+Page Up and Down, Ctrl+Shift+Page Up and Down, Ctrl+Shift+1…0, F9
//!   in place of holding Ctrl+Shift for the badges, and F8 for the active tab's folder changing
//!   (it takes a child's name).
//! - **Drops** run Strata's rules on fake data: every folder is on one volume, so a plain drop
//!   moves and Ctrl copies. Rune's binder brings every drag from outside the process, so the
//!   page needs a stand-in for one of Strata's own folder-only drags, which open tabs (X2's
//!   binder tells them by the drag's in-process payload): folders under [`OWN_DRAG`]. Any other
//!   folder is another app's, and transfers like a file.
//! - **Every tab's view stays mounted**, the inactive ones drawn under the active one at the
//!   same place, hidden by styling. A drag that a spring carries to another tab then keeps its
//!   source mounted at its path with its handlers, as P1's capture needs.

use rune::{Cx, El, Key, KeyEvent, Tx, col, el, row, text};
use strata_components::contract::dnd::{
    ActionInput, CrossDevice, DropAction, DropHover, DropOffer, Dropped, VolumeRelation,
    choose_action, uri_to_path,
};
use strata_components::tabs::{
    Accepts, BrowserTabs, BrowserTabsState, ClosedTabs, DropSite, Tab, TabId, TabsData, TabsIntent,
    Via,
};
use strata_kit::icons::IconName;

use crate::{Gallery, Scene};

/// The strip's key: tabs are `t-c7-<id>`, New tab `c7-new`.
pub const KEY: &str = "c7";

/// Dropped folders under this path stand in for Strata's own folder-only drags, which open
/// tabs (at the folder past it: `/strata-own-drag/home/demo/Projects/` opens
/// `/home/demo/Projects`). Under Rune's binder the page hears a drag's paths only at the drop
/// (U3), so before it nothing tells them apart, and no gap opens.
pub const OWN_DRAG: &str = "/strata-own-drag";

/// One fake tab.
#[derive(Clone, Debug, PartialEq)]
pub struct DemoTab {
    pub id: TabId,
    pub name: String,
    pub path: String,
    pub icon: IconName,
    pub accepts: Accepts,
}

/// C7's demo state, held in `Gallery::c7`.
pub struct Demo {
    pub tabs: Vec<DemoTab>,
    pub active: Option<TabId>,
    pub next: u64,
    pub state: BrowserTabsState,
    pub closed: ClosedTabs<DemoTab>,
    pub badges: bool,
    pub tear: bool,
    /// What the last intent did.
    pub log: String,
    /// The window title the strip asked for.
    pub title: String,
    /// The clock at the last poll.
    pub now: f64,
}

impl Default for Demo {
    fn default() -> Demo {
        Demo::with(folders(), 0)
    }
}

impl Demo {
    /// A demo over `tabs`, with tab `active` selected.
    pub fn with(tabs: Vec<DemoTab>, active: usize) -> Demo {
        Demo {
            active: tabs.get(active).map(|t| t.id),
            next: tabs.iter().map(|t| t.id.0 + 1).max().unwrap_or(1),
            tabs,
            state: BrowserTabsState::new(KEY),
            closed: ClosedTabs::new(),
            badges: false,
            tear: false,
            log: String::new(),
            title: String::new(),
            now: 0.0,
        }
    }

    fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn tab(&self, id: TabId) -> Option<&DemoTab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    fn name(&self, id: TabId) -> String {
        self.tab(id).map_or_else(String::new, |t| t.name.clone())
    }

    fn select(&mut self, id: TabId) {
        if self.tab(id).is_some() {
            self.active = Some(id);
        }
    }

    /// A new tab on `path`, at `at`.
    fn open(&mut self, name: &str, path: &str, icon: IconName, at: usize) -> TabId {
        let id = TabId(self.next);
        self.next += 1;
        let tab = DemoTab {
            id,
            name: name.to_owned(),
            path: path.to_owned(),
            icon,
            accepts: Accepts::ALL,
        };
        self.tabs.insert(at.min(self.tabs.len()), tab);
        id
    }

    /// Strata's close: the last tab closes the window (here, nothing closes and nothing is
    /// recorded); any other is recorded, and the active one hands over to its neighbour.
    fn close(&mut self, id: TabId) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        if self.tabs.len() == 1 {
            self.log = "Close window".into();
            return;
        }
        let next = self.with_data(|d| d.after_close(id));
        let tab = self.tabs.remove(index);
        self.log = format!("Close {}", tab.name);
        self.closed.record(tab, index);
        self.active = next;
    }

    fn reopen(&mut self) {
        let Some(closed) = self.closed.reopen() else {
            self.log = "Nothing to reopen".into();
            return;
        };
        let at = closed.index_in(self.tabs.len());
        let mut tab = closed.item;
        tab.id = TabId(self.next);
        self.next += 1;
        self.log = format!("Reopen {} at {at}", tab.name);
        self.active = Some(tab.id);
        self.tabs.insert(at, tab);
    }

    fn with_data<T>(&self, f: impl FnOnce(&TabsData<'_>) -> T) -> T {
        let tabs = tabs_of(&self.tabs);
        f(&TabsData::new(&tabs, self.active))
    }

    /// What Strata does with each intent.
    pub fn route(&mut self, intent: TabsIntent, tx: &mut Tx) {
        match intent {
            TabsIntent::Select { id, via } => {
                self.select(id);
                let how = match via {
                    Via::Strip => "",
                    Via::Spring => " (spring)",
                };
                self.log = format!("Select {}{how}", self.name(id));
            }
            TabsIntent::Close { id } => self.close(id),
            TabsIntent::New => {
                let (name, path, icon) = self
                    .active
                    .and_then(|id| self.tab(id))
                    .map_or(("~".into(), "/home/demo".into(), IconName::Home), |t| {
                        (t.name.clone(), t.path.clone(), t.icon)
                    });
                let id = self.open(&name, &path, icon, self.tabs.len());
                self.active = Some(id);
                self.log = format!("New tab {name}");
            }
            TabsIntent::Reorder { id, from, to } => {
                if from < self.tabs.len() && to < self.tabs.len() {
                    let tab = self.tabs.remove(from);
                    self.tabs.insert(to, tab);
                }
                self.log = format!("Move {} to {to}", self.name(id));
            }
            TabsIntent::TearOut { id, .. } => {
                self.log = format!("Tear out {}", self.name(id));
            }
            TabsIntent::TearMove { .. } | TabsIntent::DropHover { .. } => {}
            TabsIntent::DropInto { id, dropped } => self.drop_into(id, &dropped),
            TabsIntent::OpenTabs { at, dropped } => {
                let mut last = None;
                for (k, path) in dropped.data.file_paths().iter().enumerate() {
                    let text = path.to_string_lossy();
                    let text = text.strip_prefix(OWN_DRAG).unwrap_or(&text);
                    let name = text.trim_end_matches('/').rsplit('/').next().unwrap_or("/");
                    let name = name.to_owned();
                    last = Some((self.open(&name, text, IconName::Folder, at + k), name));
                }
                if let Some((id, name)) = last {
                    self.active = Some(id);
                    self.log = format!("Open {name} at {at}");
                }
            }
            TabsIntent::Title(title) => {
                tx.set_title(title.clone());
                self.title = title;
            }
        }
    }

    fn drop_into(&mut self, id: TabId, dropped: &Dropped) {
        let Some(tab) = self.tab(id).cloned() else {
            return;
        };
        let files = dropped.data.file_paths();
        let said = if !files.is_empty() {
            let action = action(&dropped.offer, dropped.mods);
            let verb = match action {
                Some(DropAction::Copy) => "Copy",
                Some(DropAction::Move) => "Move",
                Some(DropAction::Link) => "Link",
                Some(DropAction::Ask) => "Ask",
                None => return,
            };
            format!("{verb} {} into {}", files.len(), tab.path)
        } else if let Some(item) = dropped.data.items.first() {
            format!("Save {} into {}", item.mime, tab.path)
        } else {
            return;
        };
        self.select(id);
        self.log = said;
    }
}

/// The page's drop rule: one volume, so a plain drop moves and Ctrl copies.
fn action(offer: &DropOffer, mods: rune::Mods) -> Option<DropAction> {
    choose_action(ActionInput::new(
        mods,
        offer.actions,
        VolumeRelation::Same,
        CrossDevice::Ask,
    ))
}

/// The stand-in for X2's check of Strata's own folder-only drags: every dragged path is a
/// folder under [`OWN_DRAG`].
fn opens_tabs(offer: &DropOffer) -> bool {
    offer.uris.as_ref().is_some_and(|uris| {
        !uris.is_empty()
            && uris.iter().all(|u| {
                uri_to_path(u).is_some_and(|p| {
                    let p = p.to_string_lossy();
                    p.starts_with(OWN_DRAG) && p.ends_with('/')
                })
            })
    })
}

fn tabs_of(tabs: &[DemoTab]) -> Vec<Tab<'_>> {
    tabs.iter()
        .map(|t| Tab::new(t.id, t.name.as_str(), t.icon).accepts(t.accepts))
        .collect()
}

fn tab(id: u64, name: &str, path: &str, icon: IconName) -> DemoTab {
    DemoTab {
        id: TabId(id),
        name: name.into(),
        path: path.into(),
        icon,
        accepts: Accepts::ALL,
    }
}

/// Home, Documents, Music, a long name, Trash and a remote folder.
pub fn folders() -> Vec<DemoTab> {
    vec![
        tab(1, "~", "/home/demo", IconName::Home),
        tab(2, "Documents", "/home/demo/Documents", IconName::Documents),
        tab(3, "Music", "/home/demo/Music", IconName::Music),
        tab(
            4,
            "Quarterly reports and invoices 2026",
            "/home/demo/Quarterly reports and invoices 2026",
            IconName::Folder,
        ),
        DemoTab {
            accepts: Accepts::NONE,
            ..tab(5, "Trash", "trash:///", IconName::Trash)
        },
        DemoTab {
            accepts: Accepts::FILES,
            ..tab(6, "share", "sftp://nas.local/share", IconName::Network)
        },
    ]
}

/// `n` folders, for the overflowing strip.
pub fn many(n: u64) -> Vec<DemoTab> {
    (1..=n)
        .map(|i| {
            let name = format!("Folder {i:02}");
            let path = format!("/home/demo/{name}");
            tab(i, &name, &path, IconName::Folder)
        })
        .collect()
}

fn scene(g: &mut Gallery, dark: bool, demo: Demo) {
    g.open("c7", dark);
    g.c7 = demo;
}

fn one_tab(g: &mut Gallery, dark: bool) {
    scene(g, dark, Demo::with(folders()[..1].to_vec(), 0));
}

fn several(g: &mut Gallery, dark: bool) {
    scene(g, dark, Demo::with(folders(), 2));
}

fn overflow(g: &mut Gallery, dark: bool) {
    scene(g, dark, Demo::with(many(30), 21));
}

fn drop_hover(g: &mut Gallery, dark: bool) {
    let mut demo = Demo::with(folders(), 0);
    demo.state
        .set_drop_hover(DropSite::Tab(TabId(3)), DropHover::Accepting);
    scene(g, dark, demo);
}

fn new_tab_drop(g: &mut Gallery, dark: bool) {
    let mut demo = Demo::with(folders(), 0);
    demo.state.set_open_drag(true);
    scene(g, dark, demo);
}

fn badges(g: &mut Gallery, dark: bool) {
    let mut demo = Demo::with(folders(), 1);
    demo.badges = true;
    scene(g, dark, demo);
}

fn tear_out(g: &mut Gallery, dark: bool) {
    let mut demo = Demo::with(folders(), 1);
    demo.tear = true;
    scene(g, dark, demo);
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c7", dark)`, then fills in `g.c7`.
pub const SCENES: &[Scene] = &[
    ("c7-one-tab-light", |g| one_tab(g, false)),
    ("c7-one-tab-dark", |g| one_tab(g, true)),
    ("c7-several-light", |g| several(g, false)),
    ("c7-several-dark", |g| several(g, true)),
    ("c7-overflow-light", |g| overflow(g, false)),
    ("c7-overflow-dark", |g| overflow(g, true)),
    ("c7-drop-hover-light", |g| drop_hover(g, false)),
    ("c7-drop-hover-dark", |g| drop_hover(g, true)),
    ("c7-new-tab-drop-light", |g| new_tab_drop(g, false)),
    ("c7-new-tab-drop-dark", |g| new_tab_drop(g, true)),
    ("c7-badges-light", |g| badges(g, false)),
    ("c7-badges-dark", |g| badges(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens: tear-out is on, so a tab
/// dragged 36 points down off the strip tears out (logged; the page opens no window).
pub const REVIEW: &[Scene] = &[
    ("c7-tear-out-light", |g| tear_out(g, false)),
    ("c7-tear-out-dark", |g| tear_out(g, true)),
];

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c7;
    let look = &g.look;
    let k = &look.kit;
    let tabs = tabs_of(&d.tabs);
    let data = TabsData::new(&tabs, d.active);
    let mut layer = Vec::new();
    let strip = BrowserTabs::new(
        &data,
        &d.state,
        |g: &mut Gallery| &mut g.c7.state,
        |g| &mut g.c7.state.drag,
    )
    .drop_action(|g: &Gallery, id, offer, mods| {
        g.c7.tab(id)?;
        action(offer, mods)
    })
    .opens_tabs(|_, offer| opens_tabs(offer))
    .content_types(&["text/plain;charset=utf-8", "text/plain", "image/png"])
    .tear_out(d.tear)
    .badges(d.badges)
    .on_intent(|g: &mut Gallery, intent, tx| g.c7.route(intent, tx))
    .build_layered(look, cx, &mut layer);
    let panes = d.tabs.iter().fold(
        el().key("c7-panes").w_full().grow(1.0).min_h(0.0),
        |panes, t| {
            let pane = col()
                .key(format!("c7-pane-{}", t.id))
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .padding(look.space.lg)
                .gap(look.space.sm)
                .fill(k.c.bg_base)
                .focusable(d.active == Some(t.id))
                .label(format!("{} view", t.name))
                .child(text(t.path.clone()).font(k.ty.ui).color(k.c.text_muted));
            // Inactive views stay mounted where they are, under the active one, so a drag
            // from one keeps its source while a spring shows another tab (P1's rule).
            let pane = if d.active == Some(t.id) {
                pane.raise()
            } else {
                pane.opacity(0.0).a11y_hidden()
            };
            panes.child(pane)
        },
    );
    let lines = col()
        .key("c7-lines")
        .w_full()
        .padding(look.space.md)
        .gap(look.space.xs)
        .child(
            text(d.log.clone())
                .key("c7-log")
                .font(k.ty.ui)
                .color(k.c.text_base),
        )
        .child(
            text(d.title.clone())
                .key("c7-title")
                .font(k.ty.ui)
                .color(k.c.text_muted),
        );
    rune_kit::root(
        col()
            .key("c7-page")
            .w_full()
            .h_full()
            .child(row().key("c7-bar").w_full().child(strip))
            .child(panes)
            .child(lines),
        layer,
    )
}

/// Keys no focused element took while this page is open: Strata's tab shortcuts, and F9 for
/// the badges.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let d = &mut g.c7;
    let ctrl = key.mods.ctrl || key.mods.cmd;
    let shift = key.mods.shift;
    let step = |d: &Demo, delta: isize| d.with_data(|data| data.step(delta));
    match &key.key {
        Key::Command('t') if shift => d.reopen(),
        Key::Command('t') => d.route(TabsIntent::New, tx),
        Key::Command('w') => {
            if let Some(id) = d.active {
                d.close(id);
            }
        }
        Key::Command(c @ '0'..='9') if shift => {
            let n = (c.to_digit(10).unwrap_or(1) as usize + 9) % 10;
            if let Some(id) = d.with_data(|data| data.nth(n)) {
                d.select(id);
            }
        }
        Key::Tab if ctrl => {
            if let Some(id) = step(d, if shift { -1 } else { 1 }) {
                d.select(id);
            }
        }
        Key::PageDown | Key::PageUp if ctrl => {
            let delta = if key.key == Key::PageDown { 1 } else { -1 };
            if shift {
                let Some(id) = d.active else { return true };
                if let (Some(from), Some(to)) =
                    (d.index_of(id), d.with_data(|data| data.moved(id, delta)))
                {
                    let tab = d.tabs.remove(from);
                    d.tabs.insert(to, tab);
                }
            } else if let Some(id) = step(d, delta) {
                d.select(id);
            }
        }
        Key::Function(9) => d.badges = !d.badges,
        // The active tab's folder changes (a navigation): it takes the child's name.
        Key::Function(8) => {
            if let Some(t) = d
                .active
                .and_then(|id| d.tabs.iter_mut().find(|t| t.id == id))
            {
                t.name = format!("{}/Albums", t.name);
                t.path = format!("{}/Albums", t.path);
            }
        }
        _ => return false,
    }
    true
}

/// Runs at the start of every frame while this page is open, with the clock in seconds: the
/// strip's tick (its spring, the title, revealing the active tab). True when state changed.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let d = &mut g.c7;
    d.now = t;
    let polled = {
        let tabs = tabs_of(&d.tabs);
        let data = TabsData::new(&tabs, d.active);
        d.state.tick(t, &data, tx)
    };
    let changed = polled.changed || !polled.intents.is_empty();
    for intent in polled.intents {
        d.route(intent, tx);
    }
    changed
}
