//! The demo's window: a C1e list over one folder of `--root`, whose rows show the clipboard's
//! marks, and C19's footer with the Files on clipboard badge. Ctrl+C, Ctrl+X and Ctrl+V go
//! through [`FileClipboard`]; the paste is applied here (`fsys`), as Strata applies it.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use rune::{Chord, Cx, El, Key, KeyEvent, Tx, col, icon, text};
use serde_json::{Value, json};
use strata_components::collection::element::{
    Collection, CollectionState, ICON, Intent, RowMarks, Slot,
};
use strata_components::collection::selection::{CursorGone, Listing, SyncRule};
use strata_components::contract::{EntryId, EntrySource, ListSource};
use strata_components::footer::Footer;
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use crate::adapter::{FileClipboard, Mark, Paste};
use crate::fsys::{self, Ids};
use crate::log;

/// The collection's key.
pub const SCOPE: &str = "files";

static COPY: LazyLock<Chord> = LazyLock::new(|| chord("ctrl+c"));
static CUT: LazyLock<Chord> = LazyLock::new(|| chord("ctrl+x"));
static PASTE: LazyLock<Chord> = LazyLock::new(|| chord("ctrl+v"));
static UP: LazyLock<Chord> = LazyLock::new(|| chord("alt+up"));

fn chord(s: &str) -> Chord {
    Chord::parse(s).unwrap_or_else(|e| panic!("chord {s}: {e:?}"))
}

pub struct Win {
    pub root: PathBuf,
    pub folder: PathBuf,
    pub apply: bool,
    pub ids: Ids,
    pub listing: ListSource,
    pub coll: CollectionState,
    pub version: u64,
    pub look: Look,
    pub clip: FileClipboard<Win>,
    /// The badge as last logged.
    logged_badge: bool,
}

impl Win {
    pub fn new(root: PathBuf, apply: bool, clip: FileClipboard<Win>) -> Win {
        let mut coll = CollectionState::new();
        coll.controllers.pointer = Some(Box::new(
            strata_components::collection::pointer::PointerIntent::new(),
        ));
        coll.controllers.keyboard =
            Some(Box::<strata_components::collection::keyboard::Keyboard>::default());
        let mut ids = Ids::default();
        let listing = fsys::list(&mut ids, &root);
        Win {
            folder: root.clone(),
            root,
            apply,
            ids,
            listing,
            coll,
            version: 1,
            look: Look::fallback(false),
            clip,
            logged_badge: false,
        }
    }

    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root)
            .map(|r| r.display().to_string())
            .unwrap_or_else(|_| p.display().to_string())
    }

    fn path_of(&self, id: EntryId) -> Option<PathBuf> {
        self.ids.path(id).map(Path::to_path_buf)
    }

    /// The selected entries' paths, in display order.
    fn selected(&self) -> Vec<PathBuf> {
        self.coll
            .selection
            .selected_ids(&self.listing)
            .into_iter()
            .filter_map(|id| self.path_of(id))
            .collect()
    }

    /// Where Ctrl+V pastes: the one folder explicitly selected (not a load's automatic pick),
    /// else the folder shown (Strata's `paste_destination`).
    pub fn paste_destination(&self) -> PathBuf {
        let ids = self.coll.selection.selected_ids(&self.listing);
        if let [id] = ids.as_slice()
            && !self.coll.selection.is_provisional()
            && let Some(p) = self.path_of(*id)
            && p.is_dir()
        {
            // Resolved, as `open` resolves it; one outside the root the paste refuses.
            return p.canonicalize().unwrap_or(p);
        }
        self.folder.clone()
    }

    /// `p` resolved, if it is a folder inside the root.
    fn folder_in_root(&self, p: &Path) -> Option<PathBuf> {
        p.canonicalize()
            .ok()
            .filter(|p| p.starts_with(&self.root) && p.is_dir())
    }

    /// Shows `folder`, resolved, so that a link to a folder lists, marks and pastes under the
    /// paths a paste reports: false if it is no folder inside the root, as a link out of it
    /// is. Coming up from a child, that child is picked as a load's pick is.
    pub fn open(&mut self, folder: &Path, tx: &mut Tx) -> bool {
        let Some(folder) = self.folder_in_root(folder) else {
            return false;
        };
        let from = std::mem::replace(&mut self.folder, folder);
        self.listing = fsys::list(&mut self.ids, &self.folder);
        self.version += 1;
        self.coll
            .enter(SCOPE, &self.listing, None, Listing::Complete, tx);
        let child = self.ids.of(&from);
        if from.parent() == Some(self.folder.as_path())
            && let Some(i) = self.listing.index_of(child)
        {
            self.coll.selection.auto_select(&self.listing, i);
        }
        log("opened", json!({ "folder": self.rel(&self.folder) }));
        true
    }

    /// Lists the folder again, keeping what's still there selected.
    fn refresh(&mut self, tx: &mut Tx) {
        self.listing = fsys::list(&mut self.ids, &self.folder);
        self.version += 1;
        self.coll.sync(
            SCOPE,
            &self.listing,
            SyncRule::complete(CursorGone::Neighbor),
            tx,
        );
    }

    /// Selects exactly the entries named `names`, in the folder shown.
    pub fn select(&mut self, names: &[String]) {
        let at: Vec<usize> = (0..self.listing.len())
            .filter(|&i| names.iter().any(|n| n == self.listing.entry(i).name))
            .collect();
        self.coll.selection.clear();
        for (n, i) in at.into_iter().enumerate() {
            if n == 0 {
                self.coll.selection.select_only(&self.listing, i);
            } else {
                self.coll.selection.toggle(&self.listing, i);
            }
        }
    }

    fn copy(&mut self, cut: bool) {
        let paths = self.selected();
        let names: Vec<String> = paths.iter().map(|p| self.rel(p)).collect();
        match self.clip.copy(&paths, cut) {
            Ok(copy) => log(
                "copied",
                json!({ "copy": copy.get(), "cut": cut, "names": names }),
            ),
            Err(e) => log("copy-error", json!({ "error": e.to_string() })),
        }
    }

    fn paste(&mut self) {
        // Captured at the key, before another key or click can change it.
        let dest = self.paste_destination();
        let r = self
            .clip
            .paste(move |w: &mut Win, paste, tx| w.pasted(&dest, paste, tx));
        if let Err(e) = r {
            log("paste-error", json!({ "error": e.to_string() }));
        }
    }

    /// Applies what a paste read, inside the root, then shows the destination with what was
    /// pasted selected.
    fn pasted(&mut self, dest: &Path, paste: Paste, tx: &mut Tx) {
        let to = self.rel(dest);
        let (paths, moving) = match paste {
            Paste::Nothing(why) => {
                return log("paste-nothing", json!({ "why": format!("{why:?}") }));
            }
            Paste::Image(png) if !self.apply => {
                return log(
                    "paste",
                    json!({ "dest": to, "image": png.len(), "applied": false }),
                );
            }
            Paste::Image(png) => {
                match fsys::paste_image(&self.root, dest, &png) {
                    Ok(name) => {
                        log(
                            "paste",
                            json!({ "dest": to, "image": png.len(), "written": [name] }),
                        );
                        self.show(dest, &[name], tx);
                    }
                    Err(why) => log("paste-refused", json!({ "why": why })),
                }
                return;
            }
            Paste::Files {
                paths,
                moving,
                foreign,
            } => {
                if !foreign.is_empty() {
                    log("paste-foreign", json!({ "uris": foreign }));
                }
                (paths, moving)
            }
        };
        let sources: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        if !self.apply {
            return log(
                "paste",
                json!({ "dest": to, "moving": moving, "sources": sources, "applied": false }),
            );
        }
        let done = match fsys::paste(&self.root, &paths, dest, moving) {
            Ok(done) => done,
            Err(why) => return log("paste-refused", json!({ "why": why, "sources": sources })),
        };
        for (name, kept_as) in &done.conflicts {
            log("conflict", json!({ "name": name, "kept_as": kept_as }));
        }
        // A paste that failed partway still did what it did: its moves leave the cut, and the
        // destination shows it.
        let failed = done
            .failed
            .as_ref()
            .map(|(name, why)| json!({ "name": name, "why": why }));
        log(
            "paste",
            json!({
                "dest": to, "moving": moving, "sources": sources, "written": done.written,
                "skipped": done.skipped, "moved": done.moved.len(), "failed": failed,
            }),
        );
        if !done.moved.is_empty() {
            match self.clip.moved(&done.moved) {
                Ok(changed) => log("cut-consumed", json!({ "clipboard_changed": changed })),
                Err(e) => log("cut-consumed-error", json!({ "error": e.to_string() })),
            }
        }
        self.show(dest, &done.written, tx);
    }

    /// Shows `dest` with `names` selected.
    fn show(&mut self, dest: &Path, names: &[String], tx: &mut Tx) {
        if dest == self.folder {
            self.refresh(tx);
        } else if !self.open(dest, tx) {
            return;
        }
        self.select(names);
    }

    fn state(&self) -> Value {
        let marks: serde_json::Map<String, Value> = (0..self.listing.len())
            .filter_map(|i| {
                let e = self.listing.entry(i);
                let mark = self.path_of(e.id).map(|p| self.clip.mark(&p))?;
                (mark != Mark::None)
                    .then(|| (e.name.to_owned(), json!(format!("{mark:?}").to_lowercase())))
            })
            .collect();
        let selected: Vec<String> = self.selected().iter().map(|p| self.rel(p)).collect();
        json!({
            "folder": self.rel(&self.folder),
            "selected": selected,
            "provisional": self.coll.selection.is_provisional(),
            "badge": self.clip.badge(),
            "marks": marks,
        })
    }

    pub fn command(&mut self, cmd: &Value, tx: &mut Tx) {
        let names =
            |k: &str| -> Vec<String> { serde_json::from_value(cmd[k].clone()).unwrap_or_default() };
        match cmd["cmd"].as_str().unwrap_or_default() {
            "open" => {
                let to = self.root.join(cmd["path"].as_str().unwrap_or_default());
                if !self.open(&to, tx) {
                    log("bad-command", json!({ "cmd": cmd }));
                }
            }
            "select" => {
                self.select(&names("names"));
                log("selected", json!({ "names": names("names") }));
            }
            "state" => log("state", self.state()),
            "release" => match self.clip.release() {
                Ok(cleared) => log("released", json!({ "cleared": cleared })),
                Err(e) => log("release-error", json!({ "error": e.to_string() })),
            },
            other => log("bad-command", json!({ "cmd": other })),
        }
    }
}

/// The window's keys the collection passes on: the clipboard's, and Alt+Up and Backspace to
/// the parent folder.
pub fn on_key(w: &mut Win, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if COPY.matches_tx(ev, tx) {
        w.copy(false);
    } else if CUT.matches_tx(ev, tx) {
        w.copy(true);
    } else if PASTE.matches_tx(ev, tx) {
        w.paste();
    } else if UP.matches_tx(ev, tx) || (ev.key == Key::Backspace && ev.mods == Default::default()) {
        match w.folder.parent().filter(|p| p.starts_with(&w.root)) {
            Some(up) => {
                let up = up.to_path_buf();
                w.open(&up, tx);
            }
            None => return false,
        }
    } else {
        return false;
    }
    true
}

/// Logs the badge when it changes (the session tests wait for it).
pub fn poll(w: &mut Win, _: f64, _: &mut Tx) -> bool {
    let on = w.clip.badge();
    if on != w.logged_badge {
        w.logged_badge = on;
        log("badge", json!({ "on": on }));
    }
    false
}

/// The row's icon: scissors for a cut entry, the copy icon for a copied one, else its own.
fn icon_slot() -> Slot<Win> {
    let mark = |w: &Win, id: EntryId| w.path_of(id).map(|p| w.clip.mark(&p)).unwrap_or_default();
    Slot::new(
        move |w: &Win, rcx| mark(w, rcx.entry.id) as u64,
        move |w: &Win, rcx| {
            let look = rcx.look;
            let name = match mark(w, rcx.entry.id) {
                Mark::Cut => IconName::Cut,
                Mark::Copied => IconName::Copy,
                Mark::None if rcx.entry.kind.is_folder_like() => IconName::Folder,
                Mark::None => IconName::File,
            };
            icon(look.icon(name), look.icon_size.md)
                .key(ICON)
                .color(look.kit.c.icon_base)
                .a11y_hidden()
        },
    )
}

fn collection() -> Collection<Win> {
    Collection::new(
        SCOPE,
        |w: &Win| (&w.listing, &w.coll),
        |w: &mut Win| (&w.listing, &mut w.coll),
        |w: &Win| &w.look,
        |w: &Win| w.version,
    )
    .icon(icon_slot())
    .marks(|w: &Win, id| {
        let mut m = RowMarks::default();
        m.cut = w.path_of(id).is_some_and(|p| w.clip.mark(&p) == Mark::Cut);
        m
    })
    .on_intent(|w: &mut Win, intent, tx| {
        if let Intent::Activate { id, .. } = intent
            && let Some(p) = w.path_of(id).filter(|p| p.is_dir())
            && !w.open(&p, tx)
        {
            log("open-refused", json!({ "path": w.rel(&p) }));
        }
    })
}

pub fn view(w: &Win, cx: &mut Cx<Win>) -> El {
    cx.reduced_motion(!w.look.animations);
    let kit = &w.look.kit;
    let title = match w.rel(&w.folder) {
        r if r.is_empty() => "/".to_owned(),
        r => format!("/{r}"),
    };
    let header = text(title)
        .key("folder")
        .font(kit.ty.ui_medium)
        .color(kit.c.text_base)
        .px(w.look.space.lg)
        .py(w.look.space.sm);
    let list = collection().name("Files").build(w);
    let footer = Footer::new("footer")
        .clipboard(w.clip.badge())
        .build(w, &w.look);
    col()
        .key("window")
        .w_full()
        .h_full()
        .fill(kit.c.bg_base)
        .child(header)
        .child(col().w_full().grow(1.0).min_h(0.0).child(list))
        .children(footer)
        .on_layout(|_: &mut Win, r: [f32; 4]| log("layout", json!({ "w": r[2], "h": r[3] })))
        .on_window_focus(|_: &mut Win, focused: bool, _: &mut Tx| {
            log("focus", json!({ "focused": focused }));
        })
}

#[cfg(test)]
mod tests;
