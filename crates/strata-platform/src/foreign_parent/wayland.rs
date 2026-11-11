//! Wayland: `zxdg_importer_v2` on P0's queue on winit's display.
//!
//! `set_parent_of` needs winit's own `wl_surface`, so the import goes on winit's display,
//! through P0's substrate (`transfer`): its registry, queue and teardown. The imports live in
//! P0's queue state (`State::extension`), one per window key, never in a thread-local: S3's
//! thread-local import, destroyed by a TLS destructor after winit had closed the display,
//! crashed every run.
//!
//! Destroying an import undoes the parenting, so each one lives until [`release`], until the
//! window's next [`super::apply`], until a later `apply` finds its window closed, or until
//! P0's teardown.

use std::collections::HashMap;
use std::sync::Arc;

use rune::{AppHandle, WindowHandle};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::foreign::zv2::client::zxdg_imported_v2::{self, ZxdgImportedV2};
use wayland_protocols::xdg::foreign::zv2::client::zxdg_importer_v2::ZxdgImporterV2;

use super::outcome::{Fallback, Outcome, Reason, WindowSystem};
use super::parent::WaylandHandle;
use super::policy;
use crate::transfer::wayland::{Conn, State};
use crate::transfer::{Error, Status, Transfer, Turn, Unsupported, Wants};

/// Runs a window's `on_lost` in its app, through `AppHandle::update`.
pub(super) type Lost = Box<dyn FnOnce() + Send>;

/// P4's state on P0's queue.
#[derive(Default)]
struct Imports {
    importer: Option<ZxdgImporterV2>,
    /// The registry was searched and has no `zxdg_importer_v2`.
    no_importer: bool,
    /// The teardown hook is registered with this substrate.
    hooked: bool,
    by_window: HashMap<String, Import>,
}

struct Import {
    imported: ZxdgImportedV2,
    window: WindowHandle,
    /// `set_parent_of` went out: a `destroyed` now means the parent went away.
    parented: bool,
    /// The compositor sent `destroyed` before `set_parent_of` went out.
    destroyed: bool,
    lost: Option<Lost>,
}

impl Imports {
    /// Destroys `key`'s import, and any whose window is closed.
    fn sweep(&mut self, key: &str) {
        self.by_window.retain(|k, import| {
            let keep = k != key && import.window.with_raw_handle(|_| ()).is_ok();
            if !keep {
                import.imported.destroy();
            }
            keep
        });
    }

    fn destroy_all(&mut self) {
        for (_, import) in self.by_window.drain() {
            import.imported.destroy();
        }
        if let Some(importer) = self.importer.take() {
            importer.destroy();
        }
    }
}

/// Test switches, for strata-platform's own tests.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Test {
    pub(super) as_if_libwayland_below_1_23: bool,
}

/// Makes `window` (a Wayland window, key `key`) transient for the window `handle` names.
pub(super) fn apply<S: 'static>(
    window: &WindowHandle,
    app: &AppHandle<S>,
    handle: &WaylandHandle,
    lost: Option<Lost>,
    test: Test,
) -> Outcome {
    let handle_app = app.clone();
    // As P0's docs build it: false keeps the turn from redrawing.
    let turn: Turn = Arc::new(move |f| {
        handle_app.update_if(move |_, _| {
            f();
            false
        });
    });
    let wants = Wants {
        as_if_libwayland_below_1_23: test.as_if_libwayland_below_1_23,
        // Default wants otherwise: P0 binds once per process, and a clipboard attached later
        // must still find data control.
        ..Wants::default()
    };
    let transfer = match Transfer::attach(window, turn, wants) {
        Ok((t, _)) => t,
        Err(e) => return attach_failed(&e),
    };
    let Some(conn) = transfer.shared_conn() else {
        return no_shared_conn(transfer.status());
    };
    let outcome = import(&transfer, conn, window, handle, lost);
    // A parentless outcome leaves no import behind for this window.
    if !outcome.is_parented() {
        let _ = conn.with_globals(|_, _, st| st.extension::<Imports>().sweep(window.key()));
        conn.flush();
    }
    outcome
}

fn import(
    transfer: &Transfer,
    conn: &Arc<Conn>,
    window: &WindowHandle,
    handle: &WaylandHandle,
    lost: Option<Lost>,
) -> Outcome {
    let failed = |e: String| Outcome::parentless(Reason::Failed(e), Fallback::Compositor);
    if conn.is_failed() {
        return failed("winit's display connection failed".into());
    }
    let key = window.key().to_owned();
    let started = conn.with_globals(|globals, qh, st| {
        let imports = st.extension::<Imports>();
        imports.sweep(&key);
        if !imports.hooked {
            imports.hooked = true;
            // P4's objects go before P0 tears down, on the UI thread.
            transfer.before_shutdown(Box::new(teardown));
        }
        if imports.importer.is_none() && !imports.no_importer {
            match globals.bind::<ZxdgImporterV2, _, _>(qh, 1..=1, ()) {
                Ok(i) => imports.importer = Some(i),
                Err(_) => imports.no_importer = true,
            }
        }
        let importer = imports.importer.as_ref()?;
        let imported = importer.import_toplevel(handle.as_str().to_owned(), qh, key.clone());
        imports.by_window.insert(
            key.clone(),
            Import {
                imported,
                window: window.clone(),
                parented: false,
                destroyed: false,
                lost,
            },
        );
        Some(())
    });
    match started {
        None => return failed("winit's display: off the UI thread, or shut down".into()),
        Some(None) => return Outcome::parentless(Reason::NoImporter, Fallback::Compositor),
        Some(Some(())) => {}
    }
    // Every compositor sends `destroyed` for a bad handle within this round trip (S3, Q1).
    // Waiting for it first means `set_parent_of` never goes out on a dead import.
    if let Err(e) = conn.roundtrip() {
        return failed(e.to_string());
    }
    let imported = conn.with_globals(|_, _, st| {
        let imports = st.extension::<Imports>();
        match imports.by_window.get(&key) {
            Some(i) if !i.destroyed => Some(i.imported.clone()),
            _ => {
                imports.by_window.remove(&key);
                None
            }
        }
    });
    let Some(imported) = imported.flatten() else {
        return Outcome::parentless(Reason::Destroyed, Fallback::Compositor);
    };
    // Only ever the window's own toplevel surface: on any other, Mutter and Hyprland end the
    // connection (winit's) with a protocol error.
    let sent = conn.with_window_surface(window, |surface| imported.set_parent_of(surface));
    if let Err(e) = sent {
        return match e {
            Error::WindowClosed(_) => {
                Outcome::parentless(Reason::WindowClosed, Fallback::Compositor)
            }
            e => failed(e.to_string()),
        };
    }
    let _ = conn.with_globals(|_, _, st| {
        if let Some(i) = st.extension::<Imports>().by_window.get_mut(&key) {
            i.parented = true;
        }
    });
    conn.flush();
    Outcome::Parented {
        system: WindowSystem::Wayland,
        late: window.is_ready(),
    }
}

/// Destroys `key`'s import, if it has one. On the UI thread.
pub(super) fn release(key: &str) -> bool {
    let Some(transfer) = Transfer::current() else {
        return false;
    };
    let Some(conn) = transfer.shared_conn() else {
        return false;
    };
    let had = conn
        .with_globals(|_, _, st| {
            let imports = st.extension::<Imports>();
            let had = imports.by_window.contains_key(key);
            imports.sweep(key);
            had
        })
        .unwrap_or(false);
    conn.flush();
    had
}

/// Before P0's teardown, on the UI thread: nothing of P4's is left on winit's display.
fn teardown(transfer: &Transfer) {
    if let Some(conn) = transfer.shared_conn() {
        let _ = conn.with_globals(|_, _, st| st.extension::<Imports>().destroy_all());
        conn.flush();
    }
}

/// Why P0's attach failed, as an outcome.
pub(super) fn attach_failed(e: &Error) -> Outcome {
    match e {
        Error::Unsupported(Unsupported::LibwaylandBelow1_23) => policy::without_winit_display(),
        Error::Unsupported(Unsupported::NotLinux(kind)) => {
            Outcome::parentless(Reason::NotLinux(kind.clone()), Fallback::Compositor)
        }
        Error::WindowClosed(_) => Outcome::parentless(Reason::WindowClosed, Fallback::Compositor),
        e => Outcome::parentless(Reason::Failed(e.to_string()), Fallback::Compositor),
    }
}

/// P0 attached without winit's display (a data-control connection of its own only).
pub(super) fn no_shared_conn(status: &Status) -> Outcome {
    match &status.unsupported {
        Some(Unsupported::LibwaylandBelow1_23) => policy::without_winit_display(),
        Some(u) => Outcome::parentless(Reason::Failed(u.to_string()), Fallback::Compositor),
        None => Outcome::parentless(
            Reason::Failed("winit's display isn't bound".into()),
            Fallback::Compositor,
        ),
    }
}

impl Dispatch<ZxdgImporterV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZxdgImporterV2,
        _: <ZxdgImporterV2 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// `destroyed`: at import (a bad handle) or later (the parent went away). On the UI thread,
/// in P0's dispatch.
impl Dispatch<ZxdgImportedV2, String> for State {
    fn event(
        st: &mut Self,
        proxy: &ZxdgImportedV2,
        event: zxdg_imported_v2::Event,
        key: &String,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let zxdg_imported_v2::Event::Destroyed = event else {
            return;
        };
        let imports = st.extension::<Imports>();
        // A replaced or released import was destroyed already, and hears nothing more.
        let Some(import) = imports
            .by_window
            .get_mut(key)
            .filter(|i| i.imported == *proxy)
        else {
            return;
        };
        import.imported.destroy();
        if !import.parented {
            import.destroyed = true;
            return;
        }
        if let Some(import) = imports.by_window.remove(key)
            && let Some(lost) = import.lost
        {
            lost();
        }
    }
}
