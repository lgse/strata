// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

use gtk::{gdk, gio, glib, pango, prelude::*};
use vte4::prelude::*;

use crate::assets::{self, icons};

use super::theme::{SourcePalette, ThemeManager, ThemeTokens, blend};

const PANEL_HEIGHT: i32 = 240;
const MIN_PANEL_HEIGHT: i32 = 96;
const SCROLLBACK_LINES: i64 = 10_000;
/// Shown in the panel when the browser is somewhere a local shell cannot go.
const UNAVAILABLE: &str =
    "\r\n\u{1b}[7m The embedded terminal is only available for local folders. \u{1b}[0m\r\n";
const BRIGHT_MIX: f64 = 0.35;

type DirectorySource = Rc<dyn Fn() -> Option<PathBuf>>;
type ChildPid = i32;
type Generation = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpawnProgress {
    Pending,
    ExitSeen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CancellationProgress {
    Waiting(SpawnProgress),
    Child(ChildPid),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionState {
    Idle,
    Spawning {
        generation: Generation,
        progress: SpawnProgress,
    },
    Running {
        generation: Generation,
        pid: ChildPid,
    },
    Cancelling {
        generation: Generation,
        progress: CancellationProgress,
    },
    Stopping {
        generation: Generation,
        pid: ChildPid,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpawnCompletion {
    Running,
    Terminate(ChildPid),
    TerminateAndBecomeIdle(ChildPid),
    BecomeIdle,
    Stale(Option<ChildPid>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChildExit {
    Recorded,
    BecameIdle,
    Ignored,
}

// VTE's child-exited signal has no PID, so the lifecycle must drain each
// session before allowing a later generation to start.
struct SessionLifecycle {
    state: SessionState,
    next_generation: Generation,
}

impl Default for SessionLifecycle {
    fn default() -> Self {
        Self {
            state: SessionState::Idle,
            next_generation: 0,
        }
    }
}

impl SessionLifecycle {
    #[cfg(test)]
    fn state(&self) -> &SessionState {
        &self.state
    }

    fn is_idle(&self) -> bool {
        matches!(self.state, SessionState::Idle)
    }

    fn request_spawn(&mut self) -> Option<Generation> {
        if !matches!(self.state, SessionState::Idle) {
            return None;
        }
        let generation = self.next_generation.wrapping_add(1);
        self.next_generation = generation;
        self.state = SessionState::Spawning {
            generation,
            progress: SpawnProgress::Pending,
        };
        Some(generation)
    }

    fn cancel_pending_spawn(&mut self) -> Option<Generation> {
        let SessionState::Spawning {
            generation,
            progress,
        } = self.state
        else {
            return None;
        };
        self.state = SessionState::Cancelling {
            generation,
            progress: CancellationProgress::Waiting(progress),
        };
        Some(generation)
    }

    fn stop_running(&mut self) -> Option<ChildPid> {
        let SessionState::Running { generation, pid } = self.state else {
            return None;
        };
        self.state = SessionState::Stopping { generation, pid };
        Some(pid)
    }

    fn complete_spawn(
        &mut self,
        generation: Generation,
        result: Result<ChildPid, ()>,
    ) -> SpawnCompletion {
        let current = matches!(
            self.state,
            SessionState::Spawning {
                generation: active,
                ..
            }
                | SessionState::Cancelling {
                    generation: active,
                    ..
                } if active == generation
        );
        if !current {
            return SpawnCompletion::Stale(result.ok());
        }

        match (self.state, result) {
            (
                SessionState::Spawning {
                    progress: SpawnProgress::Pending,
                    ..
                },
                Ok(pid),
            ) => {
                self.state = SessionState::Running { generation, pid };
                SpawnCompletion::Running
            }
            (
                SessionState::Spawning {
                    progress: SpawnProgress::ExitSeen,
                    ..
                },
                Ok(pid),
            )
            | (
                SessionState::Cancelling {
                    progress: CancellationProgress::Waiting(SpawnProgress::ExitSeen),
                    ..
                },
                Ok(pid),
            ) => {
                self.state = SessionState::Idle;
                SpawnCompletion::TerminateAndBecomeIdle(pid)
            }
            (
                SessionState::Cancelling {
                    progress: CancellationProgress::Waiting(SpawnProgress::Pending),
                    ..
                },
                Ok(pid),
            ) => {
                self.state = SessionState::Cancelling {
                    generation,
                    progress: CancellationProgress::Child(pid),
                };
                SpawnCompletion::Terminate(pid)
            }
            (_, Err(())) => {
                self.state = SessionState::Idle;
                SpawnCompletion::BecomeIdle
            }
            _ => SpawnCompletion::Stale(result.ok()),
        }
    }

    fn child_exited(&mut self) -> ChildExit {
        match self.state {
            SessionState::Running { .. } | SessionState::Stopping { .. } => {
                self.state = SessionState::Idle;
                ChildExit::BecameIdle
            }
            SessionState::Spawning {
                generation,
                progress: SpawnProgress::Pending,
            } => {
                self.state = SessionState::Spawning {
                    generation,
                    progress: SpawnProgress::ExitSeen,
                };
                ChildExit::Recorded
            }
            SessionState::Cancelling {
                progress: CancellationProgress::Child(_),
                ..
            } => {
                self.state = SessionState::Idle;
                ChildExit::BecameIdle
            }
            SessionState::Cancelling {
                generation,
                progress: CancellationProgress::Waiting(SpawnProgress::Pending),
            } => {
                self.state = SessionState::Cancelling {
                    generation,
                    progress: CancellationProgress::Waiting(SpawnProgress::ExitSeen),
                };
                ChildExit::Recorded
            }
            SessionState::Idle
            | SessionState::Spawning {
                progress: SpawnProgress::ExitSeen,
                ..
            }
            | SessionState::Cancelling {
                progress: CancellationProgress::Waiting(SpawnProgress::ExitSeen),
                ..
            } => ChildExit::Ignored,
        }
    }
}

/// Window-scoped embedded terminal. The shell is spawned on first reveal and
/// outlives hiding, navigation, and the folder it started in.
#[derive(Clone)]
pub(super) struct TerminalPanel {
    state: Rc<PanelState>,
}

struct PanelState {
    widget: gtk::Box,
    terminal: vte4::Terminal,
    split: RefCell<Option<gtk::Paned>>,
    directory: DirectorySource,
    lifecycle: RefCell<SessionLifecycle>,
    spawn_cancellable: RefCell<Option<(Generation, gio::Cancellable)>>,
    sized: Cell<bool>,
}

impl TerminalPanel {
    pub(super) fn new(preferences: &Rc<ThemeManager>, directory: DirectorySource) -> Self {
        let terminal = vte4::Terminal::new();
        terminal.set_scrollback_lines(SCROLLBACK_LINES);
        terminal.set_scroll_on_output(true);
        terminal.set_mouse_autohide(true);
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("terminal-panel");
        widget.set_visible(false);
        widget.set_size_request(-1, MIN_PANEL_HEIGHT);

        let state = Rc::new(PanelState {
            widget,
            terminal,
            split: RefCell::new(None),
            directory,
            lifecycle: RefCell::new(SessionLifecycle::default()),
            spawn_cancellable: RefCell::new(None),
            sized: Cell::new(false),
        });
        let panel = Self { state };
        panel.state.widget.append(&panel.header());
        panel.state.widget.append(&panel.state.terminal);
        panel.bind_theme(preferences);
        let weak = Rc::downgrade(&panel.state);
        panel.state.terminal.connect_child_exited(move |_, _| {
            if let Some(state) = weak.upgrade() {
                Self { state }.child_exited();
            }
        });
        panel
    }

    pub(super) fn widget(&self) -> &gtk::Widget {
        self.state.widget.upcast_ref()
    }

    /// The panel shares the window's vertical split so it can be resized.
    pub(super) fn attach_split(&self, split: &gtk::Paned) {
        self.state.split.replace(Some(split.clone()));
    }

    #[cfg(test)]
    pub(super) fn has_session(&self) -> bool {
        !self.state.lifecycle.borrow().is_idle()
    }

    pub(super) fn is_visible(&self) -> bool {
        self.state.widget.is_visible()
    }

    pub(super) fn owns_focus(&self, focused: Option<&gtk::Widget>) -> bool {
        focused.is_some_and(|widget| {
            let terminal = self.state.terminal.upcast_ref::<gtk::Widget>();
            widget == terminal || widget.is_ancestor(terminal)
        })
    }

    /// Returns whether the panel is visible afterwards.
    pub(super) fn toggle(&self) -> bool {
        if self.is_visible() {
            self.state.widget.set_visible(false);
            return false;
        }
        self.state.widget.set_visible(true);
        self.restore_height();
        self.spawn_if_needed();
        self.state.terminal.grab_focus();
        true
    }

    /// Ends the shell and hides the panel. The next reveal starts a fresh
    /// session in whatever folder is active after the old one has exited.
    pub(super) fn close_session(&self) {
        self.shutdown();
        self.state.widget.set_visible(false);
    }

    /// Requests the shell to end while retaining lifecycle ownership until VTE
    /// reports that its child has exited.
    pub(super) fn shutdown(&self) {
        let pending = { self.state.lifecycle.borrow_mut().cancel_pending_spawn() };
        if let Some(generation) = pending {
            let cancellable = self
                .state
                .spawn_cancellable
                .borrow()
                .as_ref()
                .filter(|(active, _)| *active == generation)
                .map(|(_, cancellable)| cancellable.clone());
            if let Some(cancellable) = cancellable {
                cancellable.cancel();
            }
            return;
        }
        let running = { self.state.lifecycle.borrow_mut().stop_running() };
        if let Some(pid) = running {
            terminate(pid);
        }
    }

    fn header(&self) -> gtk::Box {
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header.add_css_class("terminal-panel-header");
        let label = gtk::Label::new(Some("Terminal"));
        label.add_css_class("terminal-panel-title");
        label.set_hexpand(true);
        label.set_xalign(0.0);
        let close = gtk::Button::builder()
            .tooltip_text("End the terminal session (F4 hides it instead)")
            .build();
        close.set_child(Some(&assets::chrome_icon(icons::X)));
        close.add_css_class("terminal-panel-close");
        close.set_cursor_from_name(Some("pointer"));
        let weak = Rc::downgrade(&self.state);
        close.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                Self { state }.close_session();
            }
        });
        header.append(&label);
        header.append(&close);
        header
    }

    fn bind_theme(&self, preferences: &Rc<ThemeManager>) {
        let terminal = self.state.terminal.clone();
        preferences.bind_preference(
            &self.state.widget,
            |manager| {
                (
                    manager.appearance_tokens(),
                    manager.appearance_source_palette(),
                )
            },
            move |_, (tokens, palette)| apply_colors(&terminal, &tokens, palette.as_ref()),
        );
        let font_terminal = self.state.terminal.clone();
        preferences.bind_preference(
            &self.state.widget,
            |manager| manager.text_size(),
            move |_, _| font_terminal.set_font_desc(interface_font().as_ref()),
        );
    }

    /// The split keeps whatever height the user dragged; only the first reveal
    /// of a window picks a size.
    fn restore_height(&self) {
        if self.state.sized.replace(true) {
            return;
        }
        if let Some(split) = self.state.split.borrow().clone() {
            let available = split.height();
            if available > PANEL_HEIGHT {
                split.set_position(available - PANEL_HEIGHT);
            }
        }
    }

    fn spawn_if_needed(&self) {
        if !self.state.lifecycle.borrow().is_idle() {
            return;
        }
        // VTE inherits Strata's own working directory when it is given none,
        // which would silently open Trash or a remote location in whatever
        // directory Strata happens to be running from.
        let Some(directory) =
            (self.state.directory)().and_then(|path| path.into_os_string().into_string().ok())
        else {
            self.state.terminal.feed(UNAVAILABLE.as_bytes());
            return;
        };
        let Some(generation) = self.state.lifecycle.borrow_mut().request_spawn() else {
            return;
        };
        // A replacement session must not open onto the dead one's scrollback.
        self.state.terminal.reset(true, true);
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let weak = Rc::downgrade(&self.state);
        let failed_shell = shell.clone();
        let cancellable = gio::Cancellable::new();
        self.state
            .spawn_cancellable
            .borrow_mut()
            .replace((generation, cancellable.clone()));
        self.state.terminal.spawn_async(
            vte4::PtyFlags::DEFAULT,
            Some(directory.as_str()),
            &[shell.as_str()],
            &[],
            glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            Some(&cancellable),
            move |result| {
                let Some(state) = weak.upgrade() else {
                    if let Ok(pid) = result {
                        terminate(pid.0);
                    }
                    return;
                };
                Self { state }.spawn_finished(generation, result, &failed_shell);
            },
        );
    }

    fn spawn_finished(
        &self,
        generation: Generation,
        result: Result<glib::Pid, glib::Error>,
        failed_shell: &str,
    ) {
        let error = result.as_ref().err().map(ToString::to_string);
        let failed = result.is_err();
        let completion = self
            .state
            .lifecycle
            .borrow_mut()
            .complete_spawn(generation, result.map(|pid| pid.0).map_err(|_| ()));
        if failed && !matches!(completion, SpawnCompletion::Stale(_)) {
            tracing::warn!(
                error = error.as_deref().unwrap_or("unknown spawn error"),
                shell = failed_shell,
                "unable to start embedded terminal"
            );
        }
        match completion {
            SpawnCompletion::Running => self.clear_spawn_cancellable(generation),
            SpawnCompletion::Terminate(pid) => {
                self.clear_spawn_cancellable(generation);
                terminate(pid);
            }
            SpawnCompletion::TerminateAndBecomeIdle(pid) => {
                self.clear_spawn_cancellable(generation);
                terminate(pid);
                self.state.widget.set_visible(false);
            }
            SpawnCompletion::BecomeIdle => {
                self.clear_spawn_cancellable(generation);
                self.state.widget.set_visible(false);
            }
            SpawnCompletion::Stale(Some(pid)) => terminate(pid),
            SpawnCompletion::Stale(None) => {}
        }
    }

    fn clear_spawn_cancellable(&self, generation: Generation) {
        let owns_callback = self
            .state
            .spawn_cancellable
            .borrow()
            .as_ref()
            .is_some_and(|(active, _)| *active == generation);
        if owns_callback {
            self.state.spawn_cancellable.borrow_mut().take();
        }
    }

    fn child_exited(&self) {
        if self.state.lifecycle.borrow_mut().child_exited() == ChildExit::BecameIdle {
            self.state.widget.set_visible(false);
        }
    }
}

fn terminate(pid: ChildPid) {
    let Some(pid) = rustix::process::Pid::from_raw(pid) else {
        return;
    };
    let _ = rustix::process::kill_process(pid, rustix::process::Signal::HUP);
}

/// Strata sets its monospace family and size on the GTK settings, so the
/// terminal follows the text-size preference by reading it back.
fn interface_font() -> Option<pango::FontDescription> {
    let name = gtk::Settings::default()?.gtk_font_name()?;
    Some(pango::FontDescription::from_string(&name))
}

fn apply_colors(terminal: &vte4::Terminal, tokens: &ThemeTokens, palette: Option<&SourcePalette>) {
    let parse = |value: &str| gdk::RGBA::parse(value).ok();
    let (Some(foreground), Some(background)) = (parse(&tokens.text), parse(&tokens.background))
    else {
        return;
    };
    // An empty palette leaves VTE's own sixteen colors in place.
    let ansi = palette
        .map(|palette| ansi_palette(tokens, palette))
        .unwrap_or_default();
    let ansi: Vec<gdk::RGBA> = ansi.iter().filter_map(|color| parse(color)).collect();
    let ansi: Vec<&gdk::RGBA> = ansi.iter().collect();
    terminal.set_colors(Some(&foreground), Some(&background), &ansi);
    terminal.set_color_cursor(parse(&tokens.accent).as_ref());
    terminal.set_color_cursor_foreground(Some(&background));
    terminal.set_color_highlight(parse(&tokens.highlight).as_ref());
    terminal.set_color_highlight_foreground(Some(&foreground));
    terminal.set_bold_is_bright(true);
}

/// The sixteen ANSI colors, drawn from the same tokens and syntax colors that
/// style the rest of Strata rather than VTE's built-in palette.
fn ansi_palette(tokens: &ThemeTokens, palette: &SourcePalette) -> Vec<String> {
    let normal = [
        tokens.background.clone(),
        tokens.danger.clone(),
        palette.string.clone(),
        palette.constant.clone(),
        tokens.accent.clone(),
        palette.statement.clone(),
        palette.type_color.clone(),
        tokens.text.clone(),
    ];
    // Bright variants move further from the background, which lightens a dark
    // theme and darkens a light one.
    let bright = [
        tokens.dim_text.clone(),
        blend(&tokens.danger, &tokens.text, BRIGHT_MIX),
        blend(&palette.string, &tokens.text, BRIGHT_MIX),
        blend(&palette.constant, &tokens.text, BRIGHT_MIX),
        blend(&tokens.accent, &tokens.text, BRIGHT_MIX),
        blend(&palette.statement, &tokens.text, BRIGHT_MIX),
        blend(&palette.type_color, &tokens.text, BRIGHT_MIX),
        tokens.text.clone(),
    ];
    normal.into_iter().chain(bright).collect()
}

#[cfg(test)]
mod tests;
