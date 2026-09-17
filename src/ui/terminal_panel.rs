// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, glib, pango, prelude::*};
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
/// How often a deferred directory change re-checks whether the shell is free.
const SYNC_RETRY: Duration = Duration::from_millis(400);

type DirectorySource = Rc<dyn Fn() -> Option<PathBuf>>;

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
    child: Cell<Option<glib::Pid>>,
    sized: Cell<bool>,
    /// The newest directory the browser asked for but the shell was not free
    /// to take. Only the newest matters; the ones it passed through do not.
    pending: RefCell<Option<PathBuf>>,
    applied: RefCell<Option<PathBuf>>,
    /// The user has typed a line the shell has not run yet.
    typed: Cell<bool>,
    retrying: Cell<bool>,
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
            child: Cell::new(None),
            sized: Cell::new(false),
            pending: RefCell::new(None),
            applied: RefCell::new(None),
            typed: Cell::new(false),
            retrying: Cell::new(false),
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
        let weak = Rc::downgrade(&panel.state);
        panel.state.terminal.connect_commit(move |_, text, _| {
            if let Some(state) = weak.upgrade() {
                state.typed.set(!line_was_ended(text));
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
        self.state.child.get().is_some()
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
    /// session in whatever folder is active then.
    pub(super) fn close_session(&self) {
        self.shutdown();
        self.state.widget.set_visible(false);
    }

    /// Ends the shell. VTE frees the PTY with the widget, and detaching it
    /// here instead would tear it out from under the still-running child.
    pub(super) fn shutdown(&self) {
        if let Some(pid) = self.state.child.take() {
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
        if self.state.child.get().is_some() {
            return;
        }
        // A replacement session must not open onto the dead one's scrollback.
        self.state.terminal.reset(true, true);
        // VTE inherits Strata's own working directory when it is given none,
        // which would silently open Trash or a remote location in whatever
        // directory Strata happens to be running from.
        let Some(directory) =
            (self.state.directory)().and_then(|path| path.into_os_string().into_string().ok())
        else {
            self.state.terminal.feed(UNAVAILABLE.as_bytes());
            return;
        };
        self.state.applied.replace(Some(PathBuf::from(&directory)));
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let weak = Rc::downgrade(&self.state);
        let failed_shell = shell.clone();
        self.state.terminal.spawn_async(
            vte4::PtyFlags::DEFAULT,
            Some(directory.as_str()),
            &[shell.as_str()],
            &[],
            glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            None::<&gtk::gio::Cancellable>,
            move |result| {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                match result {
                    Ok(pid) => state.child.set(Some(pid)),
                    Err(error) => {
                        tracing::warn!(%error, shell = %failed_shell, "unable to start embedded terminal");
                        state.widget.set_visible(false);
                    }
                }
            },
        );
    }

    fn child_exited(&self) {
        self.state.child.set(None);
        self.state.pending.take();
        self.state.applied.take();
        self.state.typed.set(false);
        self.state.widget.set_visible(false);
    }

    /// Follows the browser. The browser stays the source of truth; nothing
    /// here ever moves it.
    pub(super) fn observe_browser(&self, browser: &Rc<crate::app::Browser>) {
        let weak = Rc::downgrade(&self.state);
        browser.observe(move |_| {
            if let Some(state) = weak.upgrade() {
                Self { state }.follow();
            }
        });
    }

    fn follow(&self) {
        // No session yet: the next one spawns in the right place anyway.
        // Locations without a local path simply pause synchronisation.
        let Some(directory) = (self.state.directory)() else {
            return;
        };
        if self.state.child.get().is_none()
            || self.state.applied.borrow().as_deref() == Some(directory.as_path())
        {
            return;
        }
        self.state.pending.replace(Some(directory));
        self.apply_pending();
    }

    fn apply_pending(&self) {
        let Some(directory) = self.state.pending.borrow().clone() else {
            return;
        };
        if !self.shell_is_waiting() {
            self.retry_later();
            return;
        }
        self.state.pending.take();
        self.state.applied.replace(Some(directory.clone()));
        self.state
            .terminal
            .feed_child(change_directory(&directory).as_bytes());
    }

    /// True only when the shell itself owns the terminal and the user has not
    /// left a half-typed line at the prompt. `tcgetpgrp` cannot see the second
    /// case, so the commit signal tracks it separately.
    fn shell_is_waiting(&self) -> bool {
        if self.state.typed.get() {
            return false;
        }
        let Some(child) = self.state.child.get() else {
            return false;
        };
        let Some(pty) = self.state.terminal.pty() else {
            return false;
        };
        rustix::termios::tcgetpgrp(pty.fd())
            .is_ok_and(|group| group.as_raw_nonzero().get() == child.0)
    }

    fn retry_later(&self) {
        if self.state.retrying.replace(true) {
            return;
        }
        let weak = Rc::downgrade(&self.state);
        glib::timeout_add_local(SYNC_RETRY, move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let panel = Self { state };
            panel.apply_pending();
            if panel.state.pending.borrow().is_some() {
                return glib::ControlFlow::Continue;
            }
            panel.state.retrying.set(false);
            glib::ControlFlow::Break
        });
    }
}

/// Only a submitted or discarded line leaves the prompt empty; anything else
/// the user typed is still sitting there.
fn line_was_ended(text: &str) -> bool {
    text.contains(['\r', '\n', '\u{3}', '\u{4}', '\u{15}'])
}

fn change_directory(path: &Path) -> String {
    format!("cd -- {}\n", shell_quote(path))
}

fn shell_quote(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn terminate(pid: glib::Pid) {
    let Some(pid) = rustix::process::Pid::from_raw(pid.0) else {
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
