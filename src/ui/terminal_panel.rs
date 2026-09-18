// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
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
/// Where selected paths go in a configured agent command.
const PATHS_TOKEN: &str = "{}";

type DirectorySource = Rc<dyn Fn() -> Option<PathBuf>>;
type ChildPid = i32;
type Generation = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionKind {
    Shell,
    Agent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpawnProgress {
    Pending,
    ExitSeen(i32),
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
        kind: SessionKind,
        progress: SpawnProgress,
    },
    Running {
        generation: Generation,
        kind: SessionKind,
        pid: ChildPid,
    },
    Cancelling {
        generation: Generation,
        kind: SessionKind,
        progress: CancellationProgress,
    },
    Stopping {
        generation: Generation,
        kind: SessionKind,
        pid: ChildPid,
    },
    ExitedAgentOutput {
        status: i32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpawnCompletion {
    Running,
    Terminate(ChildPid),
    TerminateAndBecomeIdle(ChildPid),
    AgentCompleted(i32),
    BecomeIdle,
    Stale(Option<ChildPid>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChildExit {
    Recorded,
    BecameIdle,
    AgentCompleted(i32),
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

    #[cfg(test)]
    fn request_spawn(&mut self) -> Option<Generation> {
        self.request_spawn_kind(SessionKind::Shell)
    }

    fn request_spawn_kind(&mut self, kind: SessionKind) -> Option<Generation> {
        if !matches!(self.state, SessionState::Idle) {
            return None;
        }
        let generation = self.next_generation.wrapping_add(1);
        self.next_generation = generation;
        self.state = SessionState::Spawning {
            generation,
            kind,
            progress: SpawnProgress::Pending,
        };
        Some(generation)
    }

    fn cancel_pending_spawn(&mut self) -> Option<Generation> {
        let SessionState::Spawning {
            generation,
            kind,
            progress,
        } = self.state
        else {
            return None;
        };
        self.state = SessionState::Cancelling {
            generation,
            kind,
            progress: CancellationProgress::Waiting(progress),
        };
        Some(generation)
    }

    fn stop_running(&mut self) -> Option<ChildPid> {
        let SessionState::Running {
            generation,
            kind,
            pid,
        } = self.state
        else {
            return None;
        };
        self.state = SessionState::Stopping {
            generation,
            kind,
            pid,
        };
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
                    kind,
                    progress: SpawnProgress::Pending,
                    ..
                },
                Ok(pid),
            ) => {
                self.state = SessionState::Running {
                    generation,
                    kind,
                    pid,
                };
                SpawnCompletion::Running
            }
            (
                SessionState::Spawning {
                    kind: SessionKind::Agent,
                    progress: SpawnProgress::ExitSeen(status),
                    ..
                },
                Ok(_pid),
            ) => {
                self.state = SessionState::ExitedAgentOutput { status };
                SpawnCompletion::AgentCompleted(status)
            }
            (
                SessionState::Spawning {
                    progress: SpawnProgress::ExitSeen(_),
                    ..
                },
                Ok(pid),
            )
            | (
                SessionState::Cancelling {
                    progress: CancellationProgress::Waiting(SpawnProgress::ExitSeen(_)),
                    ..
                },
                Ok(pid),
            ) => {
                self.state = SessionState::Idle;
                SpawnCompletion::TerminateAndBecomeIdle(pid)
            }
            (
                SessionState::Cancelling {
                    kind,
                    progress: CancellationProgress::Waiting(SpawnProgress::Pending),
                    ..
                },
                Ok(pid),
            ) => {
                self.state = SessionState::Cancelling {
                    generation,
                    kind,
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

    #[cfg(test)]
    fn child_exited(&mut self) -> ChildExit {
        self.child_exited_with_status(0)
    }

    fn child_exited_with_status(&mut self, status: i32) -> ChildExit {
        match self.state {
            SessionState::Running {
                kind: SessionKind::Agent,
                ..
            } => {
                self.state = SessionState::ExitedAgentOutput { status };
                ChildExit::AgentCompleted(status)
            }
            SessionState::Running { .. } | SessionState::Stopping { .. } => {
                self.state = SessionState::Idle;
                ChildExit::BecameIdle
            }
            SessionState::Spawning {
                generation,
                kind,
                progress: SpawnProgress::Pending,
            } => {
                self.state = SessionState::Spawning {
                    generation,
                    kind,
                    progress: SpawnProgress::ExitSeen(status),
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
                kind,
                progress: CancellationProgress::Waiting(SpawnProgress::Pending),
            } => {
                self.state = SessionState::Cancelling {
                    generation,
                    kind,
                    progress: CancellationProgress::Waiting(SpawnProgress::ExitSeen(status)),
                };
                ChildExit::Recorded
            }
            SessionState::Idle
            | SessionState::Spawning {
                progress: SpawnProgress::ExitSeen(_),
                ..
            }
            | SessionState::Cancelling {
                progress: CancellationProgress::Waiting(SpawnProgress::ExitSeen(_)),
                ..
            }
            | SessionState::ExitedAgentOutput { .. } => ChildExit::Ignored,
        }
    }

    fn discard_completed_output(&mut self) {
        if matches!(self.state, SessionState::ExitedAgentOutput { .. }) {
            self.state = SessionState::Idle;
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
        panel.state.terminal.connect_child_exited(move |_, status| {
            if let Some(state) = weak.upgrade() {
                Self { state }.child_exited(status);
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
        self.state.lifecycle.borrow_mut().discard_completed_output();
        self.state.terminal.reset(true, true);
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
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        self.spawn(vec![shell], directory, SessionKind::Shell);
    }

    /// Starts an agent only when no terminal session currently occupies the panel.
    pub(super) fn run_agent(&self, argv: Vec<String>, directory: PathBuf) -> Result<(), String> {
        if argv.is_empty() {
            return Err("No agent command is configured.".to_string());
        }
        if !self.state.lifecycle.borrow().is_idle() {
            return Err(
                "The embedded terminal already has a session. Close it with the \u{00d7} in the panel, or type exit, and run the agent again."
                    .to_string(),
            );
        }
        let directory = exact(&directory)?;
        self.state.widget.set_visible(true);
        self.restore_height();
        self.spawn(argv, directory, SessionKind::Agent);
        self.state.terminal.grab_focus();
        Ok(())
    }

    fn spawn(&self, argv: Vec<String>, directory: String, kind: SessionKind) {
        let Some(generation) = self.state.lifecycle.borrow_mut().request_spawn_kind(kind) else {
            return;
        };
        // A new session must not open onto stale scrollback.
        self.state.terminal.reset(true, true);
        let arguments: Vec<&str> = argv.iter().map(String::as_str).collect();
        let weak = Rc::downgrade(&self.state);
        let program = argv.first().cloned().unwrap_or_default();
        let cancellable = gio::Cancellable::new();
        let callback_cancellable = cancellable.clone();
        self.state
            .spawn_cancellable
            .borrow_mut()
            .replace((generation, cancellable.clone()));
        self.state.terminal.spawn_async(
            vte4::PtyFlags::DEFAULT,
            Some(directory.as_str()),
            &arguments,
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
                Self { state }.spawn_finished(
                    generation,
                    kind,
                    result,
                    &program,
                    callback_cancellable.is_cancelled(),
                );
            },
        );
    }

    fn spawn_finished(
        &self,
        generation: Generation,
        kind: SessionKind,
        result: Result<glib::Pid, glib::Error>,
        program: &str,
        cancelled: bool,
    ) {
        let error = result.as_ref().err().map(ToString::to_string);
        let spawn_error = result.as_ref().err().cloned();
        let failed = result.is_err();
        let completion = self
            .state
            .lifecycle
            .borrow_mut()
            .complete_spawn(generation, result.map(|pid| pid.0).map_err(|_| ()));
        if failed && !matches!(completion, SpawnCompletion::Stale(_)) {
            tracing::warn!(
                error = error.as_deref().unwrap_or("unknown spawn error"),
                %program,
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
            SpawnCompletion::AgentCompleted(status) => {
                self.clear_spawn_cancellable(generation);
                self.state.terminal.feed(exit_notice(status).as_bytes());
            }
            SpawnCompletion::BecomeIdle => {
                self.clear_spawn_cancellable(generation);
                if kind == SessionKind::Agent && !cancelled {
                    if let Some(error) = spawn_error.as_ref() {
                        self.state
                            .terminal
                            .feed(spawn_failure(program, error).as_bytes());
                    }
                } else {
                    self.state.widget.set_visible(false);
                }
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

    fn child_exited(&self, status: i32) {
        match self
            .state
            .lifecycle
            .borrow_mut()
            .child_exited_with_status(status)
        {
            ChildExit::BecameIdle => self.state.widget.set_visible(false),
            ChildExit::AgentCompleted(status) => {
                self.state.terminal.feed(exit_notice(status).as_bytes());
            }
            ChildExit::Recorded | ChildExit::Ignored => {}
        }
    }
}

fn terminate(pid: ChildPid) {
    let Some(pid) = rustix::process::Pid::from_raw(pid) else {
        return;
    };
    let _ = rustix::process::kill_process(pid, rustix::process::Signal::HUP);
}

/// VTE reports a failed execution long after the menu has closed, so the panel
/// is the only place left to say so.
fn spawn_failure(program: &str, error: &glib::Error) -> String {
    let message = error.message();
    format!(
        "\r\n\u{1b}[7m {program} could not be started: {message} \u{1b}[0m\r\n\u{1b}[7m Press F4 to close this panel. \u{1b}[0m\r\n"
    )
}

/// Shown in the panel itself, where the agent's own output is still visible.
fn exit_notice(status: i32) -> String {
    let signalled = status & 0x7f;
    let describe = if signalled == 0 {
        format!("exited with status {}", (status >> 8) & 0xff)
    } else {
        format!("was terminated by signal {signalled}")
    };
    format!("\r\n\u{1b}[7m The agent {describe}. Press F4 to close this panel. \u{1b}[0m\r\n")
}

/// Splits the configured command into words and places selected paths.
/// Native paths are only ever passed through exactly: VTE spawns from UTF-8
/// argv, so a path it cannot represent stops the launch instead of reaching
/// the agent as a different path.
pub(super) fn agent_argv(
    command: &str,
    directory: &Path,
    paths: &[PathBuf],
) -> Result<Vec<String>, String> {
    exact(directory)?;

    let words = glib::shell_parse_argv(command)
        .map_err(|error| format!("The configured agent command could not be read: {error}"))?;
    let mut argv: Vec<String> = Vec::with_capacity(words.len());
    for word in &words {
        argv.push(exact(Path::new(word))?);
    }
    let Some(program) = argv.first() else {
        return Err("No agent command is configured. Set one in Settings.".to_string());
    };
    argv[0] = resolve_program(program, directory)?;
    let text: Vec<String> = paths
        .iter()
        .map(|path| exact(path))
        .collect::<Result<_, _>>()?;
    Ok(place_paths(argv, &text))
}

/// A path that is not valid UTF-8 cannot cross the spawn boundary unchanged,
/// and changing it would point the agent at a different file.
fn exact(path: &Path) -> Result<String, String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        format!(
            "\u{201c}{}\u{201d} cannot be passed to a program without changing it",
            path.display()
        )
    })
}

/// VTE spawns without G_SPAWN_SEARCH_PATH, so the program is resolved here.
/// A relative command resolves against the folder the agent will run in, not
/// against whatever directory Strata itself was started from.
fn resolve_program(program: &str, directory: &Path) -> Result<String, String> {
    let path = Path::new(program);
    if path.is_absolute() {
        return is_executable_file(path)
            .then(|| program.to_owned())
            .ok_or_else(|| format!("\u{201c}{program}\u{201d} is not an executable file"));
    }
    if program.contains('/') {
        let candidate =
            std::path::absolute(directory.join(path)).unwrap_or_else(|_| directory.join(path));
        return is_executable_file(&candidate)
            .then(|| exact(&candidate))
            .transpose()?
            .ok_or_else(|| {
                format!(
                    "\u{201c}{program}\u{201d} is not an executable file in {}",
                    directory.display()
                )
            });
    }
    let resolved = super::terminal::find_on_path(std::env::var_os("PATH").as_deref(), program)
        .ok_or_else(|| format!("\u{201c}{program}\u{201d} was not found on your PATH"))?;
    exact(Path::new(&resolved))
}

fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|data| data.is_file() && data.permissions().mode() & 0o111 != 0)
}

/// A bare `{}` word becomes one argument per path. Inside a word it produces
/// one textual argument, and without a placeholder paths follow the command.
fn place_paths(argv: Vec<String>, text: &[String]) -> Vec<String> {
    if !argv.iter().skip(1).any(|word| word.contains(PATHS_TOKEN)) {
        return argv.into_iter().chain(text.iter().cloned()).collect();
    }
    let mut placed = Vec::with_capacity(argv.len() + text.len());
    for (index, word) in argv.into_iter().enumerate() {
        match () {
            _ if index == 0 || !word.contains(PATHS_TOKEN) => placed.push(word),
            _ if word == PATHS_TOKEN => placed.extend(text.iter().cloned()),
            _ => placed.push(word.replace(PATHS_TOKEN, &text.join(" "))),
        }
    }
    placed
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
