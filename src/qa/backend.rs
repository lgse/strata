// SPDX-License-Identifier: MIT

//! GTK's side of the QA mode: what `strata_qa::Backend` asks, answered from widgets.

use std::{
    cell::{OnceCell, RefCell},
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

use gtk::{gdk, gio, glib, prelude::*};

use super::{
    POLL_INTERVAL, READY_POLL, READY_TIMEOUT, Sample, criticals,
    input::{Event, Injector},
    next_paint,
    protocol::{
        Backend, Chord, Command, EntryRow, FieldState, Labels, Mark, Modifier, Orientation,
        Outcome, Predicate, Rect, Scope, Scroll, States, Verdict,
    },
    settle, state, targets,
};
use crate::ui::accessibility_qa as names;

pub(super) enum Host {
    Application(gtk::Application),
    Portal(glib::MainLoop),
}

pub(super) struct Gtk {
    pub(super) host: Host,
    /// Connected once the window is ready, before the first step.
    pub(super) injector: OnceCell<Injector>,
    /// A floating chooser becomes the active window while open; state is read from the browser.
    pub(super) main: glib::WeakRef<gtk::Window>,
    /// Captures, text measurements and instants that GTK's own steps keep under a name.
    pub(super) samples: RefCell<HashMap<String, Sample>>,
    /// Owns the clipboard a `clipboard_set` set, until the `clipboard` read closes it.
    pub(super) clipboard: RefCell<Option<gdk::Display>>,
}

impl Gtk {
    pub(super) fn new(host: Host) -> Self {
        Self {
            host,
            injector: OnceCell::new(),
            main: glib::WeakRef::new(),
            samples: RefCell::default(),
            clipboard: RefCell::default(),
        }
    }

    pub(super) fn injector(&self) -> Result<&Injector, String> {
        self.injector
            .get()
            .ok_or_else(|| "no input connection".to_owned())
    }

    /// Keeps the application running while it has no window yet.
    pub(super) fn hold(&self) -> Option<gio::ApplicationHoldGuard> {
        match &self.host {
            Host::Application(application) => Some(application.hold()),
            Host::Portal(_) => None,
        }
    }

    async fn await_ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            let window = self.main_window();
            if let Some(window) = window.as_ref().filter(|window| state::is_ready(window)) {
                self.main.set(Some(window));
                settle(Some(window)).await;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("the Strata window never showed an entry list".to_owned());
            }
            match window.as_ref().and_then(WidgetExt::frame_clock) {
                Some(clock) => next_paint(&clock, POLL_INTERVAL).await,
                None => glib::timeout_future(READY_POLL).await,
            }
        }
    }

    pub(super) fn main_window(&self) -> Option<gtk::Window> {
        self.main.upgrade().or_else(|| match &self.host {
            Host::Application(application) => application
                .active_window()
                .or_else(|| application.windows().into_iter().next()),
            Host::Portal(_) => None,
        })
    }

    pub(super) fn windows(&self) -> Vec<gtk::Window> {
        match &self.host {
            Host::Application(application) => application.windows(),
            Host::Portal(_) => gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|window| window.downcast::<gtk::Window>().ok())
                .collect(),
        }
    }
}

fn rect(bounds: &gtk::graphene::Rect) -> Rect {
    Rect {
        x: f64::from(bounds.x()),
        y: f64::from(bounds.y()),
        width: f64::from(bounds.width()),
        height: f64::from(bounds.height()),
    }
}

impl Backend for Gtk {
    type Node = gtk::Widget;
    type Kept = glib::WeakRef<gtk::Widget>;

    async fn ready(&self) -> Result<(), String> {
        match self.host {
            Host::Application(_) => self.await_ready().await?,
            Host::Portal(_) => {}
        }
        let _ = self.injector.set(Injector::connect()?);
        Ok(())
    }

    fn quit(&self) {
        match &self.host {
            Host::Application(application) => application.quit(),
            Host::Portal(main_loop) => main_loop.quit(),
        }
    }

    fn windowless(&self) -> bool {
        matches!(self.host, Host::Portal(_))
    }

    fn criticals(&self) -> Vec<String> {
        criticals::take()
    }

    fn window(&self) -> Option<gtk::Widget> {
        self.main_window().map(Cast::upcast)
    }

    fn toplevels(&self) -> Vec<gtk::Widget> {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter(|window| window.is::<gtk::Window>())
            .collect()
    }

    fn title(&self, window: &gtk::Widget) -> Option<String> {
        window
            .downcast_ref::<gtk::Window>()?
            .title()
            .map(|title| title.to_string())
    }

    fn window_width(&self, window: &gtk::Widget) -> i32 {
        window.width()
    }

    /// AT-SPI reports no focus for a focused row a list keeps unmapped while it is
    /// scrolled away, so neither does this.
    fn focus(&self, window: &gtk::Widget) -> Option<gtk::Widget> {
        gtk::prelude::GtkWindowExt::focus(window.downcast_ref::<gtk::Window>()?)
            .filter(|focus| focus.has_focus() && focus.is_mapped())
    }

    fn parent(&self, node: &gtk::Widget) -> Option<gtk::Widget> {
        node.parent()
    }

    fn children(&self, node: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut children = Vec::new();
        let mut child = node.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            children.push(current);
        }
        children
    }

    fn visible(&self, node: &gtk::Widget) -> bool {
        node.is_visible()
    }

    fn attached(&self, node: &gtk::Widget) -> bool {
        node.root().is_some()
    }

    fn role(&self, node: &gtk::Widget) -> &'static str {
        targets::role_name(node)
    }

    fn name(&self, node: &gtk::Widget) -> String {
        targets::accessible_name(node)
    }

    fn description(&self, node: &gtk::Widget) -> String {
        targets::accessible_description(node)
    }

    /// An entry's mark is recorded on its `GtkListItem`, which is no widget: `entry` and
    /// `entries` find the row that stands for it.
    fn mark(&self, node: &gtk::Widget) -> Option<Mark> {
        names::naming(node).and_then(|naming| naming.mark)
    }

    fn states(&self, node: &gtk::Widget) -> States {
        targets::states(node)
    }

    fn orientation(&self, node: &gtk::Widget) -> Option<Orientation> {
        match node
            .dynamic_cast_ref::<gtk::Orientable>()
            .map(OrientableExt::orientation)
        {
            Some(gtk::Orientation::Horizontal) => Some(Orientation::Horizontal),
            Some(gtk::Orientation::Vertical) => Some(Orientation::Vertical),
            _ => None,
        }
    }

    fn text(&self, node: &gtk::Widget) -> Option<String> {
        targets::text_content(node)
    }

    fn field(&self, node: &gtk::Widget) -> Option<FieldState> {
        state::read_field(node)
    }

    fn value(&self, node: &gtk::Widget) -> Option<f64> {
        node.downcast_ref::<gtk::Range>().map(|range| range.value())
    }

    fn frame(&self, node: &gtk::Widget) -> Option<Rect> {
        state::rendered_bounds(node).as_ref().map(rect)
    }

    /// Popovers are their own native surfaces, hence the surface offset and screen origin.
    fn origin(&self, node: &gtk::Widget) -> Result<(f64, f64), String> {
        let native = node.native().ok_or("the target has no native surface")?;
        let surface = native
            .surface()
            .ok_or("the target's surface is not realized")?;
        let (offset_x, offset_y) = native.surface_transform();
        let (origin_x, origin_y) = self.injector()?.surface_origin(&surface)?;
        Ok((
            f64::from(origin_x) + offset_x,
            f64::from(origin_y) + offset_y,
        ))
    }

    fn layout(&self, node: &gtk::Widget, window: &gtk::Widget) -> Option<Rect> {
        node.compute_bounds(window).as_ref().map(rect)
    }

    fn scroll(&self, node: &gtk::Widget, along: Orientation) -> Option<Scroll> {
        let scroller = node.downcast_ref::<gtk::ScrolledWindow>()?;
        let adjustment = match along {
            Orientation::Horizontal => scroller.hadjustment(),
            Orientation::Vertical => scroller.vadjustment(),
        };
        Some(Scroll {
            value: adjustment.value(),
            page: adjustment.page_size(),
            scrollable: adjustment.upper() - adjustment.page_size() > adjustment.lower(),
        })
    }

    fn scroll_position(&self, scroller: &gtk::Widget) -> Option<f64> {
        targets::presented(scroller)
            .into_iter()
            .find_map(|widget| {
                state::rendered_bounds(&widget)?;
                widget.downcast::<gtk::Scrollbar>().ok()
            })
            .map(|scrollbar| scrollbar.adjustment().value())
    }

    fn entry(&self, node: &gtk::Widget) -> Option<EntryRow<gtk::Widget>> {
        state::row(node)
    }

    fn entries(&self, container: &gtk::Widget) -> Vec<EntryRow<gtk::Widget>> {
        state::container_rows(container)
    }

    fn labels(&self) -> Labels {
        Labels {
            preview: crate::i18n::tr(crate::ui::PREVIEW_LABEL),
            peek: crate::i18n::tr(super::PEEK_LABEL),
            search_results: crate::i18n::tr(crate::ui::SEARCH_RESULTS_LABEL),
        }
    }

    fn settings_file(&self) -> PathBuf {
        crate::storage::config_directory().join("settings.toml")
    }

    fn keep(&self, node: &gtk::Widget) -> Self::Kept {
        node.downgrade()
    }

    fn recall(&self, kept: &Self::Kept) -> Option<gtk::Widget> {
        kept.upgrade()
    }

    fn dump(&self, window: &gtk::Widget) -> String {
        window
            .downcast_ref::<gtk::Window>()
            .map(state::dump)
            .unwrap_or_default()
    }

    async fn settle(&self, window: &gtk::Widget) {
        settle(window.downcast_ref::<gtk::Window>()).await;
    }

    async fn sleep(&self, time: Duration) {
        glib::timeout_future(time).await;
    }

    fn check(&self, scope: &Scope<'_, Self>, predicate: &Predicate) -> Option<Verdict> {
        self.check_own(scope, predicate).transpose()
    }

    fn begin(&self, window: &gtk::Widget) -> Result<(), String> {
        let injector = self.injector()?;
        if injector.logging() {
            let origin = window
                .downcast_ref::<gtk::Window>()
                .and_then(NativeExt::surface)
                .ok_or_else(|| "the window has no surface".to_owned())
                .and_then(|surface| injector.surface_origin(&surface))?;
            injector.set_log_origin(origin);
        }
        Ok(())
    }

    fn adopt(&self, window: &gtk::Widget) {
        self.main.set(window.downcast_ref::<gtk::Window>());
    }

    async fn resize(
        &self,
        window: &gtk::Widget,
        width: u16,
        height: Option<u16>,
    ) -> Result<(), String> {
        let injector = self.injector()?;
        let height = height.map_or_else(
            || u16::try_from(window.height()).map_err(|error| error.to_string()),
            Ok,
        );
        window
            .downcast_ref::<gtk::Window>()
            .and_then(NativeExt::surface)
            .ok_or_else(|| "the window has no surface".to_owned())
            .and_then(|surface| injector.resize(&surface, width, height?))
    }

    async fn chord(&self, window: &gtk::Widget, chord: &Chord) -> Result<(), String> {
        self.press(window, chord).await
    }

    async fn text_input(&self, window: &gtk::Widget, character: char) -> Result<(), String> {
        self.type_character(window, character).await
    }

    async fn hold(&self, window: &gtk::Widget, modifiers: &[Modifier]) -> Result<(), String> {
        self.hold_keys(window, modifiers).await.map(|_| ())
    }

    async fn release(&self, window: &gtk::Widget, modifiers: &[Modifier]) {
        if let Ok(held) = self.modifier_keycodes(modifiers) {
            self.release_keys(window, &held).await;
        }
    }

    async fn key(&self, name: &str, pressed: bool) -> Result<(), String> {
        let keycode = self.named_keycode(name)?;
        self.injector()?.send(Event::Key { keycode, pressed })
    }

    async fn pointer_move(&self, x: i16, y: i16) -> Result<(), String> {
        self.injector()?.send(Event::Motion { x, y })
    }

    fn pointer_position(&self) -> Result<(i16, i16), String> {
        self.injector()?.pointer_position()
    }

    async fn button(&self, number: u8, pressed: bool) -> Result<(), String> {
        self.injector()?.send(Event::Button { number, pressed })
    }

    fn activate(&self, node: &gtk::Widget) -> bool {
        node.activate()
    }

    async fn command(&self, scope: &Scope<'_, Self>, command: &Command) -> Option<Outcome> {
        self.command_own(scope, command).await
    }
}
