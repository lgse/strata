// SPDX-License-Identifier: MIT

//! Rune's side of the QA mode: what `strata_qa::Backend` asks, answered from the
//! accessibility tree of the host it drives (`qa::host`): the offscreen host, or a real
//! window on the session host.

use std::{
    cell::Cell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use strata_core::{i18n::tr, logging::criticals, storage};
use strata_qa::{
    Backend, Chord, Command, FieldState, Labels, Mark, Modifier, Orientation, Outcome, Rect, Scope,
    Scroll, States,
};

use super::{
    field, filter,
    host::{Activated, Host},
    input_log::InputLog,
    invariants,
    staged::Staged,
    tree::{Kept, Node, Seen, Snapshot},
};
use crate::{App, host::offscreen::Offscreen, shell};

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const READY_POLL: Duration = Duration::from_millis(5);
/// The names of the preview drawer, the folder peek and the search results, as the GTK
/// application's `PREVIEW_LABEL`, `PEEK_LABEL` and `SEARCH_RESULTS_LABEL` spell them. The
/// slice draws none of the three.
const PREVIEW: &str = "Preview";
const PEEK: &str = "Folder peek";
const SEARCH_RESULTS: &str = "Search results";

pub(super) struct Rune<H: Host = Staged> {
    host: H,
    quitting: Cell<bool>,
}

impl Rune {
    pub(super) fn new(host: Offscreen, log: InputLog) -> Self {
        Self::on(Staged::new(host, log))
    }

    /// Opens a folder, as the launch does with its argument.
    #[cfg(test)]
    pub(super) fn open(&self, folder: &std::path::Path) {
        let location = strata_core::model::Location::local(folder);
        self.host.app(|app| app.open(location));
    }

    /// The window's state, for a test to reach behind the steps.
    #[cfg(test)]
    pub(super) fn browser(&self) -> Rc<strata_fs::app::Browser> {
        self.host.app(|app| app.browser.clone())
    }

    /// One turn of the host while no step runs. Returns whether it did anything.
    pub(super) fn turn(&self) -> bool {
        self.host.turn()
    }

    /// Blocks until the host has work, and for `limit` at most.
    pub(super) fn wait(&self, limit: Duration) {
        self.host.pause(limit);
    }
}

impl<H: Host> Rune<H> {
    pub(super) fn on(host: H) -> Self {
        Self {
            host,
            quitting: Cell::new(false),
        }
    }

    pub(super) fn quitting(&self) -> bool {
        self.quitting.get()
    }

    fn snapshot(&self) -> Rc<Snapshot> {
        self.host.snapshot()
    }

    fn read<T>(&self, node: &Node, read: impl FnOnce(&Snapshot, &Seen) -> T) -> Option<T> {
        let snapshot = self.snapshot();
        snapshot.get(node).map(|seen| read(&snapshot, seen))
    }
}

/// Whether the folder the launch opened has been listed, or could not be.
fn listed(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.probe.listed)(app))
}

/// Whether the view shown is the one the saved `browser_mode` names, which the runner
/// writes for the run's mode. The shell shows the first view when none answers to it, so a
/// run of a view not mounted would otherwise pass on that one.
fn shows_saved_mode(app: &App) -> Result<(), String> {
    let mode = &shell::preferences(app).browser_mode;
    match shell::shown_view(app) {
        Some(view) if (view.shows)(mode) => Ok(()),
        Some(view) => Err(format!(
            "the run starts in the {mode:?} view, and the window shows the {} view instead: \
             no view mounted answers to {mode:?}",
            view.probe.name
        )),
        None => Err("the Strata window shows no view".to_owned()),
    }
}

impl<H: Host> Backend for Rune<H> {
    type Node = Node;
    type Kept = Kept;

    async fn ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + READY_TIMEOUT;
        // The view is chosen when the window is built, so a fallback is named at once rather
        // than waited on until it lists.
        self.host.with_app(shows_saved_mode)??;
        loop {
            self.host.settle().await;
            if self.host.with_app(listed)? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("the Strata window never showed an entry list".to_owned());
            }
            self.host.pause(READY_POLL);
        }
    }

    fn quit(&self) {
        self.quitting.set(true);
    }

    fn criticals(&self) -> Vec<String> {
        criticals::take()
    }

    fn window(&self) -> Option<Node> {
        self.snapshot().root().and_then(|root| root.node.clone())
    }

    /// The one window is the only toplevel.
    fn toplevels(&self) -> Vec<Node> {
        self.window().into_iter().collect()
    }

    fn title(&self, _window: &Node) -> Option<String> {
        self.host.with_app(|app| app.title().to_owned()).ok()
    }

    fn window_width(&self, window: &Node) -> i32 {
        let width = self.frame(window).map_or(0.0, |frame| frame.width);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a window is far narrower than i32::MAX points"
        )]
        let width = width.round() as i32;
        width
    }

    fn focus(&self, _window: &Node) -> Option<Node> {
        self.snapshot().focus().and_then(|focus| focus.node.clone())
    }

    fn parent(&self, node: &Node) -> Option<Node> {
        let snapshot = self.snapshot();
        let parent = snapshot.at(snapshot.get(node)?.parent?)?;
        parent.node.clone()
    }

    fn children(&self, node: &Node) -> Vec<Node> {
        let snapshot = self.snapshot();
        let Some(seen) = snapshot.get(node) else {
            return Vec::new();
        };
        let children = seen.children.iter();
        children
            .filter_map(|child| snapshot.at(*child)?.node.clone())
            .collect()
    }

    /// The tree holds no hidden element: Rune leaves those out.
    fn visible(&self, node: &Node) -> bool {
        self.attached(node)
    }

    fn attached(&self, node: &Node) -> bool {
        self.snapshot().get(node).is_some()
    }

    fn role(&self, node: &Node) -> &'static str {
        self.read(node, |_, seen| seen.role()).unwrap_or("filler")
    }

    fn name(&self, node: &Node) -> String {
        self.read(node, |_, seen| seen.name.clone())
            .unwrap_or_default()
    }

    fn description(&self, node: &Node) -> String {
        self.read(node, |_, seen| seen.description.clone())
            .unwrap_or_default()
    }

    fn mark(&self, node: &Node) -> Option<Mark> {
        self.read(node, |_, seen| seen.mark).flatten()
    }

    fn states(&self, node: &Node) -> States {
        self.read(node, |snapshot, seen| States {
            selected: seen.has_state("selected"),
            focused: snapshot.is_focus(seen),
            checked: seen.has_state("checked"),
            pressed: false,
            sensitive: !seen.has_state("disabled"),
            selectable: seen.has_state("selected") || seen.has_state("unselected"),
            editable: field::is_one(&seen.role) && !seen.has_state("read-only"),
            focusable: seen.has_action("focus"),
            busy: false,
            marked: filter::marked(snapshot.outlined(), seen),
        })
        .unwrap_or_default()
    }

    fn orientation(&self, _node: &Node) -> Option<Orientation> {
        None
    }

    fn text(&self, node: &Node) -> Option<String> {
        self.read(node, |_, seen| match seen.role.as_str() {
            "label" => Some(seen.name.clone()),
            role if field::is_one(role) => seen.value.clone(),
            _ => None,
        })
        .flatten()
    }

    fn field(&self, node: &Node) -> Option<FieldState> {
        self.read(node, |snapshot, seen| {
            field::state(seen, snapshot.is_focus(seen))
        })
        .flatten()
    }

    fn value(&self, node: &Node) -> Option<f64> {
        self.read(node, |_, seen| seen.number).flatten()
    }

    fn frame(&self, node: &Node) -> Option<Rect> {
        self.read(node, |_, seen| seen.bounds).flatten()
    }

    /// Rune draws a window and its menus on one surface.
    fn origin(&self, _node: &Node) -> Result<(f64, f64), String> {
        self.host.origin()
    }

    fn layout(&self, node: &Node, _window: &Node) -> Option<Rect> {
        self.frame(node)
    }

    /// A scroll view's, as Rune's tree reports it (`tree::Scrolled`).
    fn scroll(&self, node: &Node, along: Orientation) -> Option<Scroll> {
        let scrolled = self.read(node, |_, seen| seen.scroll).flatten()?;
        Some(scrolled.along(along))
    }

    fn scroll_position(&self, scroller: &Node) -> Option<f64> {
        let scrolled = self.read(scroller, |_, seen| seen.scroll).flatten()?;
        scrolled.bar()
    }

    fn labels(&self) -> Labels {
        Labels {
            preview: tr(PREVIEW),
            peek: tr(PEEK),
            search_results: tr(SEARCH_RESULTS),
        }
    }

    fn settings_file(&self) -> PathBuf {
        storage::config_directory().join("settings.toml")
    }

    fn keep(&self, node: &Node) -> Kept {
        self.snapshot().keep(node).after(self.host.replacements())
    }

    /// A row its column has been replaced in since is another row, as on GTK.
    fn recall(&self, kept: &Kept) -> Option<Node> {
        let replaced = kept
            .entry()
            .is_some_and(|entry| self.host.replaced_since(entry, kept.since()));
        if replaced {
            return None;
        }
        self.snapshot().recall(kept)
    }

    fn dump(&self, _window: &Node) -> String {
        self.snapshot().dump()
    }

    async fn settle(&self, _window: &Node) {
        self.host.settle().await;
    }

    async fn sleep(&self, time: Duration) {
        self.host.sleep(time).await;
    }

    /// There is one window, and it is the one the steps act on already.
    fn adopt(&self, _window: &Node) {}

    async fn resize(&self, window: &Node, width: u16, height: Option<u16>) -> Result<(), String> {
        let frame = self.frame(window).ok_or("the window is not drawn")?;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a window's height in points fits f32"
        )]
        let height = height.map_or(frame.height as f32, f32::from);
        self.host.resize(f32::from(width), height).await
    }

    async fn chord(&self, _window: &Node, chord: &Chord) -> Result<(), String> {
        self.host.chord(chord).await
    }

    async fn text_input(&self, _window: &Node, character: char) -> Result<(), String> {
        self.host.text_input(character).await
    }

    async fn hold(&self, _window: &Node, modifiers: &[Modifier]) -> Result<(), String> {
        self.host.hold(modifiers).await
    }

    async fn release(&self, _window: &Node, modifiers: &[Modifier]) {
        self.host.release(modifiers).await;
    }

    async fn key(&self, name: &str, pressed: bool) -> Result<(), String> {
        self.host.key(name, pressed).await
    }

    async fn pointer_move(&self, x: i16, y: i16) -> Result<(), String> {
        self.host.pointer_move(x, y).await
    }

    fn pointer_position(&self) -> Result<(i16, i16), String> {
        Ok(self.host.pointer_position())
    }

    async fn button(&self, number: u8, pressed: bool) -> Result<(), String> {
        self.host.button(number, pressed).await
    }

    /// The element's press handler, through the click an assistive client asks for.
    fn activate(&self, node: &Node) -> bool {
        let target = self.read(node, |_, seen| {
            let key = seen.key.clone().filter(|_| seen.has_action("click"))?;
            Some(Activated {
                key,
                bounds: seen.bounds,
            })
        });
        target
            .flatten()
            .is_some_and(|target| self.host.activate(&target))
    }

    /// The two steps of Rune's own: the keyboard layout, which the GTK application reads from
    /// its display server, and the invariants ([`invariants`]), which it does not check.
    async fn command(&self, scope: &Scope<'_, Self>, command: &Command) -> Option<Outcome> {
        match command {
            Command::Keymap(keymap) => Some(Outcome::from(self.host.set_layout(keymap))),
            Command::Invariants => {
                let observation = scope.scene().observation;
                let focused = scope.focused().is_some();
                let failed = self
                    .host
                    .with_app(move |app| invariants::check(app, &observation, focused));
                Some(match failed {
                    Ok(failed) => Outcome {
                        result: serde_json::json!(failed),
                        ..Outcome::default()
                    },
                    Err(error) => Outcome::from(Err::<(), _>(error)),
                })
            }
            _ => None,
        }
    }
}
