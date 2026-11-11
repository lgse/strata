//! The gallery page for C6 LocationBar: its scenes, demo state, view and key and poll hooks. Scene
//! names start with `c6-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/C6-location-bar.md`.
//!
//! The page draws Strata's header with the location bar in its middle, a stand-in file pane
//! (`c6-files`, focusable and pressable, its label counting presses) and a `c6-log` line naming
//! the last thing the host did, which scripts read. The demo plays Strata's part over a small
//! fake folder tree under `/home/demo`:
//!
//! - it lists suggestions as Strata's planner does (prefix, `~`, relative, the folder's own
//!   children when it matches alone), on a fake worker: an answer comes due 0.05 s after its
//!   query on the virtual clock, or 1.5 s when it lists a folder named `nas` (a slow share).
//!   It answers even a cancelled query, so the bar's own stale check is what scripts see;
//! - it opens typed paths that exist, and logs "Unable to open location" for the rest;
//! - page keys stand in for the window's: Ctrl+L edits the location, and F5 reloads the
//!   folder 0.3 s later (a load that ends while the entry is open). F7 completes as Tab does,
//!   as a key added to Strata's `focus_next` would.

use rune::{Chord, Cx, El, Key, KeyEvent, Tx, col, row, text};
use strata_components::contract::dnd::DropHover;
use strata_components::header::{self, Header};
use strata_components::location_bar::{
    Candidate, Completions, Crumb, LocationBar, LocationBarState, LocationIntent, Polled, Trail,
    Via,
};
use strata_components::scroll::BarLook;
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use crate::{Gallery, Scene};

const HOME: &str = "/home/demo";
const KEY: &str = "c6-loc";
/// How long the fake worker takes to list a folder, and a slow share.
const FAST: f64 = 0.05;
const SLOW: f64 = 1.5;

/// C6's demo state, held in `Gallery::c6`.
pub struct Demo {
    pub bar: LocationBarState,
    /// The open folder, absolute.
    pub cwd: String,
    pub trail: Trail,
    /// What the host did last, shown in `c6-log`.
    pub log: String,
    /// Presses on the file pane.
    pub files_pressed: u32,
    /// The clock at the last poll.
    pub now: f64,
    /// The fake worker: answers, each due at a clock time.
    pub worker: Vec<(f64, Completions)>,
    /// Folder loads due at a clock time (F5).
    pub loads: Vec<f64>,
    pub complete_keys: Vec<Chord>,
    /// The bar sits in a file chooser: a middle click opens nothing.
    pub chooser: bool,
}

impl Default for Demo {
    fn default() -> Demo {
        let cwd = format!("{HOME}/Projects/strata");
        Demo {
            bar: LocationBarState::new(),
            trail: trail(&cwd),
            cwd,
            log: String::new(),
            files_pressed: 0,
            now: 0.0,
            worker: Vec::new(),
            loads: Vec::new(),
            complete_keys: ["tab", "shift+tab", "f7"]
                .into_iter()
                .filter_map(|c| Chord::parse(c).ok())
                .collect(),
            chooser: false,
        }
    }
}

/// The fake tree: each folder's child folders.
fn children(path: &str) -> Option<&'static [&'static str]> {
    Some(match path {
        "/" => &["etc", "home", "usr"],
        "/home" => &["demo"],
        "/home/demo" => &[
            ".config",
            "Desktop",
            "Documents",
            "Downloads",
            "Music",
            "Pictures",
            "Projects",
            "Videos",
            "nas",
        ],
        "/home/demo/Documents" => &["Invoices", "Letters", "Notes"],
        "/home/demo/Projects" => &["rune", "strata", "strata-components", "website"],
        "/home/demo/Projects/strata" => &["docs", "src", "tests"],
        "/home/demo/nas" => &["archive", "backups"],
        "/usr" => &["bin", "lib", "share"],
        "/usr/share" => &["applications", "fonts", "icons"],
        "/etc"
        | "/home/demo/.config"
        | "/home/demo/Desktop"
        | "/home/demo/Downloads"
        | "/home/demo/Music"
        | "/home/demo/Pictures"
        | "/home/demo/Videos"
        | "/home/demo/Documents/Invoices"
        | "/home/demo/Documents/Letters"
        | "/home/demo/Documents/Notes"
        | "/home/demo/Projects/rune"
        | "/home/demo/Projects/strata-components"
        | "/home/demo/Projects/website"
        | "/home/demo/Projects/strata/docs"
        | "/home/demo/Projects/strata/src"
        | "/home/demo/Projects/strata/tests"
        | "/home/demo/nas/archive"
        | "/home/demo/nas/backups"
        | "/usr/bin"
        | "/usr/lib"
        | "/usr/share/applications"
        | "/usr/share/fonts"
        | "/usr/share/icons" => &[],
        _ => return None,
    })
}

/// `path` as Strata displays it: home shortened to `~`.
fn display(path: &str) -> String {
    if path == HOME {
        "~".into()
    } else if let Some(rest) = path.strip_prefix(&format!("{HOME}/")) {
        format!("~/{rest}")
    } else {
        path.to_owned()
    }
}

/// The crumbs for `path`: from `~` under home, else from `/`.
fn trail(path: &str) -> Trail {
    let mut crumbs = Vec::new();
    let rest = if path == HOME || path.starts_with(&format!("{HOME}/")) {
        crumbs.push(Crumb::new(HOME, "~").description("~").icon(IconName::Home));
        path[HOME.len()..].trim_start_matches('/')
    } else {
        crumbs.push(
            Crumb::new("/", "/")
                .description("/")
                .icon(IconName::Computer)
                .joins_next(true),
        );
        path.trim_start_matches('/')
    };
    let mut at = crumbs[0].id.trim_end_matches('/').to_owned();
    for seg in rest.split('/').filter(|s| !s.is_empty()) {
        at = format!("{at}/{seg}");
        crumbs.push(Crumb::new(at.clone(), seg).description(display(&at)));
    }
    Trail::new(crumbs, display(path))
}

/// A typed location, absolute, as Strata resolves it (`~`, `~/`, absolute, else relative to
/// the open folder); `..` and `.` resolve as text.
fn resolve(text: &str, cwd: &str) -> String {
    let t = text.trim();
    let joined = if t == "~" {
        HOME.to_owned()
    } else if let Some(r) = t.strip_prefix("~/") {
        format!("{HOME}/{r}")
    } else if t.starts_with('/') {
        t.to_owned()
    } else {
        format!("{cwd}/{t}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

/// A folder to list: where, the leaf typed so far, and what each suggestion starts with.
struct Listing {
    folder: String,
    leaf: String,
    prepend: String,
    /// The typed folder itself, listed instead when at most one sibling matches it.
    inside: Option<(String, String)>,
}

/// Strata's `plan_completions`, over the fake tree. `None`: nothing to suggest.
fn plan(text: &str, cwd: &str) -> Option<Result<Vec<Candidate>, Listing>> {
    let input = text.trim();
    let split = |base: &str, typed: &str, prepend_base: &str| -> Listing {
        let (dir, leaf) = match typed.rsplit_once('/') {
            Some((d, l)) => (d, l),
            None => ("", typed),
        };
        let folder = if dir.is_empty() {
            base.to_owned()
        } else {
            resolve(dir, base)
        };
        let prepend = if dir.is_empty() {
            prepend_base.to_owned()
        } else {
            format!("{prepend_base}{dir}/")
        };
        let inside = (!typed.is_empty() && !typed.ends_with('/')).then(|| {
            (
                resolve(typed, base),
                format!("{prepend_base}{}/", typed.trim_end_matches('/')),
            )
        });
        Listing {
            folder,
            leaf: leaf.to_owned(),
            prepend,
            inside,
        }
    };
    if input.is_empty() {
        return Some(Err(Listing {
            folder: cwd.to_owned(),
            leaf: String::new(),
            prepend: String::new(),
            inside: None,
        }));
    }
    if input == "~" {
        return Some(Ok(vec![
            Candidate::new("~/", "~/")
                .matched(std::iter::once(0..1))
                .hint("home"),
        ]));
    }
    if let Some(rel) = input.strip_prefix("~/") {
        return Some(Err(split(HOME, rel, "~/")));
    }
    if input.starts_with('~') {
        return None;
    }
    if let Some(abs) = input.strip_prefix('/') {
        return Some(Err(split("/", abs, "/")));
    }
    Some(Err(split(cwd, input, &format!("{cwd}/"))))
}

fn list(folder: &str, leaf: &str, prepend: &str) -> Vec<Candidate> {
    let Some(names) = children(folder) else {
        return Vec::new();
    };
    let lower = leaf.to_lowercase();
    let mut out: Vec<Candidate> = names
        .iter()
        .filter(|n| leaf.starts_with('.') || !n.starts_with('.'))
        .filter(|n| n.to_lowercase().starts_with(&lower))
        .map(|n| {
            let label = format!("{n}/");
            let matched = n
                .char_indices()
                .nth(leaf.chars().count())
                .map_or(n.len(), |(i, _)| i);
            Candidate::new(label, format!("{prepend}{n}/"))
                .matched(std::iter::once(0..matched))
                .hint(display(folder))
        })
        .collect();
    out.sort_by_key(|c| c.label.to_lowercase());
    out
}

/// The suggestions for `text` and how long listing them takes.
fn suggest(text: &str, cwd: &str) -> (Vec<Candidate>, f64) {
    match plan(text, cwd) {
        None => (Vec::new(), FAST),
        Some(Ok(ready)) => (ready, 0.0),
        Some(Err(l)) => {
            let mut slow = l.folder.ends_with("/nas");
            let mut found = list(&l.folder, &l.leaf, &l.prepend);
            if found.len() <= 1
                && let Some((inside, prepend)) = &l.inside
            {
                let kids = list(inside, "", prepend);
                if !kids.is_empty() {
                    slow |= inside.ends_with("/nas");
                    found = kids;
                }
            }
            (found, if slow { SLOW } else { FAST })
        }
    }
}

impl Demo {
    /// Strata's part, faked.
    fn apply(&mut self, intent: LocationIntent, tx: &mut Tx) {
        match intent {
            LocationIntent::Complete(q) => {
                let (found, wait) = suggest(&q.text, &self.cwd);
                self.worker
                    .push((self.now + wait, Completions::new(q.generation, found)));
            }
            // The fake worker finishes what it started, as a listing already running does: its
            // late answer then reaches the bar, which drops it as stale.
            LocationIntent::CancelCompletion => {}
            LocationIntent::Submit { text } => {
                let path = resolve(&text, &self.cwd);
                if children(&path).is_some() {
                    self.go(path, "Open", tx);
                } else {
                    self.log = format!("Unable to open location {}", text.trim());
                }
            }
            LocationIntent::Open { id, via } => {
                let how = match via {
                    Via::Click => "Open",
                    Via::Menu => "Open from menu",
                    Via::Spring => "Spring open",
                };
                self.go(id, how, tx);
            }
            LocationIntent::OpenInNewTab { id } => self.log = format!("New tab {}", display(&id)),
            LocationIntent::OpenInNewWindow { id } => {
                self.log = format!("New window {}", display(&id));
            }
            LocationIntent::Cancelled => self.log = "Cancelled".into(),
            LocationIntent::CopyPath => {
                tx.copy(display(&self.cwd));
                self.log = format!("Copied {}", display(&self.cwd));
            }
            LocationIntent::DropInto { id, dropped } => {
                self.log = format!(
                    "Drop {} into {}",
                    dropped.data.file_paths().len(),
                    display(&id)
                );
            }
            // Hover reports change nothing here, so the log keeps the last action.
            LocationIntent::DropHover { .. } => {}
            _ => {}
        }
    }

    fn route(&mut self, polled: Polled, tx: &mut Tx) -> bool {
        let changed = polled.changed;
        for intent in polled.intents {
            self.apply(intent, tx);
        }
        changed
    }

    fn go(&mut self, path: String, how: &str, tx: &mut Tx) {
        self.log = format!("{how} {}", display(&path));
        self.trail = trail(&path);
        self.cwd = path;
        let polled = self.bar.navigated();
        self.route(polled, tx);
    }

    /// The suggestions for `text`, delivered at once (a scene's).
    fn fill(&mut self, text: &str) {
        let polled = self.bar.begin_edit(text);
        for intent in polled.intents {
            if let LocationIntent::Complete(q) = intent {
                let (found, _) = suggest(&q.text, &self.cwd);
                let _ = self.bar.deliver(Completions::new(q.generation, found));
            }
        }
        self.bar.edit.set_caret(text.len());
    }
}

fn bar_state(g: &mut Gallery) -> &mut LocationBarState {
    &mut g.c6.bar
}

fn scene(g: &mut Gallery, dark: bool) {
    g.open("c6", dark);
    g.c6 = Demo::default();
}

fn at(g: &mut Gallery, dark: bool, path: &str) {
    scene(g, dark);
    g.c6.cwd = path.to_owned();
    g.c6.trail = trail(path);
}

/// A deep path, with a crumb longer than 32 characters: it overflows and scrolls to its end.
const DEEP: &str = "/home/demo/Projects/strata/src/ui/browser/location/breadcrumbs/a-rather-long-folder-name-to-shorten-in-the-middle/deeper/still-deeper/even-deeper/leaf";

fn editing(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    // The entry on the open folder, its text selected; no answer is in yet.
    let text = g.c6.trail.text.clone();
    let _ = g.c6.bar.begin_edit(&text);
}

fn completion(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    g.c6.fill("~/D");
    g.c6.bar.set_highlighted(Some(1));
}

fn drop_hover(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    g.c6.bar.set_drop_hover(1, DropHover::Accepting);
}

/// The Copy path button just after a press: its check and "Path copied".
fn copied(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    g.c6.bar.show_copied(0.0);
}

fn menu(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    // Opened where a secondary press 160 points in would open it: at the press's x, its top on
    // the bar's bottom (the bar is centred in the header). Scenes can't press; the scripts
    // `hierarchy-menu` and `hierarchy-menu-opens` open it with a real secondary press.
    let look = &g.look;
    let header = look.frame.bar + 2.0 * look.frame.inset;
    let bar = BarLook::app(look).horizontal();
    let height = look.frame.control + bar.thickness + 2.0 * bar.margin;
    g.c6.bar.open_menu(160.0, (header + height) / 2.0);
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c6", dark)`, then fills in `g.c6`.
pub const SCENES: &[Scene] = &[
    ("c6-short-light", |g| scene(g, false)),
    ("c6-short-dark", |g| scene(g, true)),
    ("c6-deep-light", |g| at(g, false, DEEP)),
    ("c6-deep-dark", |g| at(g, true, DEEP)),
    ("c6-system-light", |g| at(g, false, "/usr/share/icons")),
    ("c6-system-dark", |g| at(g, true, "/usr/share/icons")),
    ("c6-editing-light", |g| editing(g, false)),
    ("c6-editing-dark", |g| editing(g, true)),
    ("c6-completion-light", |g| completion(g, false)),
    ("c6-completion-dark", |g| completion(g, true)),
    ("c6-drop-hover-light", |g| drop_hover(g, false)),
    ("c6-drop-hover-dark", |g| drop_hover(g, true)),
    ("c6-menu-light", |g| menu(g, false)),
    ("c6-menu-dark", |g| menu(g, true)),
    ("c6-copied-light", |g| copied(g, false)),
    ("c6-copied-dark", |g| copied(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[];

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    cx.reduced_motion(!look.animations);
    let d = &g.c6;
    let mut layer = Vec::new();
    let bar = LocationBar::new(
        KEY,
        look,
        &d.trail,
        &d.bar,
        bar_state,
        |g: &mut Gallery, i, tx| g.c6.apply(i, tx),
    )
    .complete_keys(&d.complete_keys)
    .open_in_new(!d.chooser)
    .build(cx, &mut layer);
    let header = Header::new("c6-header", look)
        .leading(stand_in(
            look,
            IconName::Sidebar,
            "Show sidebar",
            "c6-sidebar",
        ))
        .center(bar)
        .action(stand_in(look, IconName::Plus, "New tab", "c6-new-tab"))
        .build();
    let k = &look.kit;
    let files = col()
        .key("c6-pane")
        .grow(1.0)
        .w_full()
        .min_h(0.0)
        .fill(k.c.layer_01)
        .padding(look.space.lg)
        .gap(look.space.sm)
        .child(
            row()
                .key("c6-files")
                .role("list")
                .label(format!("Files pressed {}", d.files_pressed))
                .focusable(true)
                .on_press(|g: &mut Gallery| g.c6.files_pressed += 1)
                .border(1.0, k.c.border_weak)
                .radius(k.radius.md)
                .w_full()
                .h(look.row.list * 4.0),
        )
        .child(
            text(if d.log.is_empty() {
                "—".to_owned()
            } else {
                d.log.clone()
            })
            .key("c6-log")
            .font(k.ty.small)
            .color(k.c.text_muted),
        );
    rune_kit::root(
        col().key("c6").w_full().h_full().child(header).child(files),
        layer,
    )
}

/// A header action that does nothing here: the page draws the bar's neighbours.
fn stand_in(look: &Look, icon: IconName, label: &str, key: &str) -> El {
    header::action(look, icon, label)
        .on_press(|_: &mut Gallery| {})
        .build(&look.kit)
        .key(key.to_owned())
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let d = &mut g.c6;
    let ctrl_l = matches!(&key.key, Key::Command('l'))
        || (key.mods.ctrl && matches!(&key.key, Key::Text(t) if t == "l"));
    if ctrl_l {
        let text = d.trail.text.clone();
        let polled = d.bar.begin_edit(&text);
        d.route(polled, tx);
        return true;
    }
    if key.key == Key::Function(5) {
        d.loads.push(d.now + 0.3);
        return true;
    }
    false
}

/// Runs at the start of every frame while this page is open, with the clock in seconds: the
/// bar's tick, the fake worker's due answers and due folder loads. True when state changed.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let d = &mut g.c6;
    d.now = t;
    let polled = d.bar.tick(t, tx);
    let mut changed = d.route(polled, tx);
    let (due, later): (Vec<_>, Vec<_>) = d.worker.drain(..).partition(|(at, _)| *at <= t);
    d.worker = later;
    for (_, answer) in due {
        let polled = d.bar.deliver(answer);
        changed |= d.route(polled, tx);
    }
    if !d.worker.is_empty() || !d.loads.is_empty() {
        tx.wake_after(std::time::Duration::from_millis(16));
    }
    let before = d.loads.len();
    d.loads.retain(|at| *at > t);
    if d.loads.len() != before {
        let polled = d.bar.navigated();
        d.log = format!("Loaded {}", d.trail.text);
        d.route(polled, tx);
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests;
