// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use strata_core::{i18n::tr, logging::criticals, storage};
use strata_qa::{
    Backend, Chord, Command, DEFAULT_WAIT, EntryPart, Failure, FieldState, Labels, Mark, Modifier,
    Orientation, Outcome, Rect, Scope, Scroll, States, TextMatch,
};

use super::{
    field, filter,
    host::{Activated, Host},
    input_log::InputLog,
    invariants,
    staged::Staged,
    tree::{Kept, Node, Seen, Snapshot},
    windows::Windows,
};
use crate::{App, host::offscreen::Offscreen, shell};

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const READY_POLL: Duration = Duration::from_millis(5);
const PREVIEW: &str = "Preview";
const PEEK: &str = "Folder peek";
const SEARCH_RESULTS: &str = "Search results";

pub(super) struct Rune<H: Host = Windows> {
    host: H,
    quitting: Cell<bool>,
}

impl Rune {
    /// The browser window on the offscreen host, and the floating chooser it opens.
    pub(super) fn new(host: Offscreen, log: InputLog) -> Self {
        let size = host.size();
        Self::on(Windows::new(Staged::new(host, log.clone()), log, size))
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

    /// The modifiers the shell holds as held down in the window ([`shell::held_modifiers`]).
    #[cfg(test)]
    pub(super) fn held_modifiers(&self) -> rune::Mods {
        self.host.app(shell::held_modifiers)
    }

    #[cfg(test)]
    pub(super) fn update(&self, act: impl FnOnce(&mut App, &mut rune::Tx)) {
        self.host.update(act);
    }

    pub(super) fn flush_at_exit(&self) {
        self.host
            .update(|app, _| crate::folder_views::flush_at_exit(app));
        strata_core::storage::writer::flush_at_exit();
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

    pub(super) fn idle_turn(&self) -> bool {
        self.host.idle_turn()
    }

    pub(super) fn pause(&self, limit: Duration) {
        self.host.pause(limit);
    }

    fn snapshot(&self) -> Rc<Snapshot> {
        self.host.snapshot()
    }

    fn read<T>(&self, node: &Node, read: impl FnOnce(&Snapshot, &Seen) -> T) -> Option<T> {
        let snapshot = self.snapshot();
        snapshot.get(node).map(|seen| read(&snapshot, seen))
    }

    async fn set_scrollbar(&self, orientation: Orientation, value: f64) -> Result<(), String> {
        let (node, key, offset) = self
            .snapshot()
            .scrollbar_target(orientation)
            .ok_or_else(|| format!("no {orientation:?} scroll bar"))?;
        let [mut x, mut y] = offset;
        match orientation {
            Orientation::Horizontal => x = value,
            Orientation::Vertical => y = value,
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a scroll offset in window points fits f32"
        )]
        self.host
            .scroll_to((&node, &key), x as f32, y as f32)
            .await?;
        let actual = self
            .scroll(&node, orientation)
            .map(|scroll| scroll.value)
            .ok_or_else(|| format!("the {orientation:?} scroll bar left the tree"))?;
        // Rune's accessibility dump rounds scroll offsets to tenths of a point.
        if (actual - value).abs() > 0.5 {
            return Err(format!(
                "the {orientation:?} scroll bar took {actual} instead of {value}"
            ));
        }
        Ok(())
    }
}

/// Whether the folder the launch opened has been listed, or could not be.
fn listed(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.probe.listed)(app))
}

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
        // The portal host has no window until a request opens one: `adopt` waits for that.
        if self.host.windowless() {
            return Ok(());
        }
        let deadline = Instant::now() + READY_TIMEOUT;
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

    fn windowless(&self) -> bool {
        self.host.windowless()
    }

    fn nests_dialogs(&self) -> bool {
        true
    }

    fn window(&self) -> Option<Node> {
        self.host.window()
    }

    fn toplevels(&self) -> Vec<Node> {
        self.host.toplevels()
    }

    fn title(&self, window: &Node) -> Option<String> {
        self.host.title_of(window)
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

    fn focus(&self, window: &Node) -> Option<Node> {
        let focus = self.snapshot().focus()?.node.clone()?;
        focus.is_within(window).then_some(focus)
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
            pressed: seen.role() == "toggle button" && seen.has_state("checked"),
            sensitive: !seen.has_state("disabled"),
            selectable: seen.has_state("selected") || seen.has_state("unselected"),
            editable: field::is_one(&seen.role) && !seen.has_state("read-only"),
            focusable: snapshot.is_focusable(seen),
            busy: seen.busy || seen.has_state("busy"),
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

    fn entry_part_frame(&self, node: &Node, part: EntryPart) -> Option<Rect> {
        self.snapshot().entry_part_frame(node, part)
    }

    /// Rune draws a window and its menus on one surface.
    fn origin(&self, node: &Node) -> Result<(f64, f64), String> {
        self.host.origin(node)
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
    fn adopt(&self, window: &Node) {
        self.host.adopt(window);
    }

    async fn activate_window(
        &self,
        _window: &Node,
        _deadline: std::time::Instant,
    ) -> Result<bool, String> {
        Err(
            "activate_window needs a host that drives several browser windows; the Rune QA \
             hosts drive one"
                .to_owned(),
        )
    }

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
                node: node.clone(),
                key,
                bounds: seen.bounds,
            })
        });
        target
            .flatten()
            .is_some_and(|target| self.host.activate(&target))
    }

    async fn command(&self, scope: &Scope<'_, Self>, command: &Command) -> Option<Outcome> {
        match command {
            Command::Keymap(keymap) => Some(Outcome::from(self.host.set_layout(keymap))),
            Command::Reveal { target } => Some(self.reveal(scope, target).await),
            Command::RevealControl { target } => Some(self.reveal_control(scope, target).await),
            Command::SetScrollbar { orientation, value } => Some(Outcome::from(
                self.set_scrollbar(*orientation, *value).await,
            )),
            Command::Invariants => {
                let observation = scope.scene().observation;
                let focused = scope.focused().is_some();
                let failed = self
                    .host
                    .with_app(move |app| invariants::check(app, &observation, focused));
                Some(match failed {
                    Ok(failed) => Outcome {
                        result: json!(failed),
                        ..Outcome::default()
                    },
                    Err(error) => Outcome::from(Err::<(), _>(error)),
                })
            }
            Command::ClipboardSet(content) => {
                self.host.set_clipboard(content).await.map(Outcome::from)
            }
            Command::Clipboard {
                text,
                mime,
                message,
            } => {
                let mut result = self.host.read_clipboard(mime.as_deref()).await?;
                if matches!(&result, Ok(Some(got)) if !text.matches(got)) {
                    let deadline = Instant::now() + Duration::from_secs(1);
                    while Instant::now() < deadline {
                        self.host.sleep(Duration::from_millis(20)).await;
                        result = self.host.read_clipboard(mime.as_deref()).await?;
                        if !matches!(&result, Ok(Some(got)) if !text.matches(got)) {
                            break;
                        }
                    }
                }
                let check = ClipboardCheck {
                    text,
                    mime: mime.as_deref(),
                    message: message.clone(),
                };
                Some(clipboard_outcome(scope, check, result))
            }
            _ => None,
        }
    }
}

fn abridged(text: &str) -> String {
    const LIMIT: usize = 200;
    match text.char_indices().nth(LIMIT) {
        Some((end, _)) => format!("{}… ({} characters)", &text[..end], text.chars().count()),
        None => text.to_owned(),
    }
}

struct ClipboardCheck<'a> {
    text: &'a TextMatch,
    mime: Option<&'a str>,
    message: Option<String>,
}

fn clipboard_outcome<B: Backend>(
    scope: &Scope<'_, B>,
    check: ClipboardCheck<'_>,
    result: Result<Option<String>, String>,
) -> Outcome {
    let predicate = json!({ "clipboard": check.text, "mime": check.mime });
    match result {
        Err(error) => Outcome::error(error),
        Ok(Some(read)) if check.text.matches(&read) => Outcome::default(),
        Ok(Some(read)) => Outcome {
            failures: vec![Failure {
                predicate,
                actual: json!(abridged(&read)),
                message: check.message,
            }],
            error: Some("the clipboard holds other text".to_owned()),
            ..Outcome::default()
        },
        Ok(None) => {
            let failure = Failure {
                predicate,
                actual: Value::Null,
                message: check.message.or_else(|| {
                    Some("selected table text on the private display clipboard".to_owned())
                }),
            };
            scope.timed_out(failure, DEFAULT_WAIT)
        }
    }
}
