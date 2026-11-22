// SPDX-License-Identifier: MIT

//! Wayland implementation of compositor backdrop blur on the shared `transfer` queue.
//!
//! Binds `ext_background_effect_manager_v1` first (standard cross-compositor staging protocol),
//! waits one round trip for its initial `capabilities` event, and falls back to
//! `org_kde_kwin_blur_manager` when either the global is absent or `Capability::Blur` is not
//! advertised. All proxies live in `State::extension::<BlurState>()` and are destroyed by the
//! `before_shutdown` hook before winit closes its `wl_display`, never from a thread-local
//! destructor or on winit's `wl_surface`.

use std::collections::HashMap;
use std::sync::Arc;

use rune::{AppHandle, WindowHandle};
use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::{self, Capability, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};
use wayland_protocols_plasma::blur::client::{
    org_kde_kwin_blur::OrgKdeKwinBlur, org_kde_kwin_blur_manager::OrgKdeKwinBlurManager,
};

use super::Protocol;
use crate::transfer::wayland::{State, with_surface};
use crate::transfer::{Transfer, Turn, Wants};

/// Logical extent passed to `wl_region.add(0, 0, w, h)` for full-surface blur under
/// `ext_background_effect_v1`. The specification guarantees the compositor clips the blur region
/// to the surface bounds; `1_000_000` covers any desktop surface while staying safely below
/// `i32` / `wl_fixed_t` overflow limits even under high scale factors.
pub(super) const FULL_SURFACE_EXTENT: i32 = 1_000_000;

/// Returns whether `flags` includes [`Capability::Blur`], tolerating unknown future bitfield
/// flags that `wayland-client` decodes as `WEnum::Unknown`.
pub(super) fn capabilities_include_blur(flags: WEnum<Capability>) -> bool {
    let raw = match flags {
        WEnum::Value(caps) => caps.bits(),
        WEnum::Unknown(bits) => bits,
    };
    Capability::from_bits_truncate(raw).contains(Capability::Blur)
}

pub(super) enum Manager {
    Ext {
        manager: ExtBackgroundEffectManagerV1,
        /// Updated by `ext_background_effect_manager_v1::Event::Capabilities`. Starts `false`
        /// until the compositor confirms `Capability::Blur` in the initial round trip.
        blur_supported: bool,
    },
    Kde(OrgKdeKwinBlurManager),
    Unavailable,
}

impl Manager {
    pub(super) fn protocol(&self) -> Protocol {
        match self {
            Self::Ext { .. } => Protocol::ExtBackgroundEffect,
            Self::Kde(_) => Protocol::KdeBlur,
            Self::Unavailable => Protocol::None,
        }
    }

    pub(super) fn destroy(self) {
        if let Self::Ext { manager, .. } = self {
            manager.destroy();
        }
    }
}

pub(super) enum SurfaceEffect {
    Ext(ExtBackgroundEffectSurfaceV1),
    Kde(OrgKdeKwinBlur),
}

impl SurfaceEffect {
    pub(super) fn destroy(self) {
        match self {
            Self::Ext(effect) => effect.destroy(),
            Self::Kde(blur) => blur.release(),
        }
    }
}

struct SurfaceEntry {
    window: WindowHandle,
    /// Raw `wl_surface` pointer when `effect` was created, so a recreated native surface under
    /// the same window key never reuses a stale effect proxy.
    surface_ptr: usize,
    /// Latest requested state for this window, used to reconcile live surfaces when the
    /// compositor dynamically restores `Capability::Blur`.
    desired: bool,
    effect: Option<SurfaceEffect>,
}

#[derive(Default)]
struct BlurState {
    manager: Option<Manager>,
    hooked: bool,
    by_window: HashMap<String, SurfaceEntry>,
}

impl BlurState {
    fn protocol(&self) -> Protocol {
        self.manager
            .as_ref()
            .map_or(Protocol::None, Manager::protocol)
    }

    fn sweep_closed(&mut self) {
        self.by_window.retain(|_, entry| {
            let live = entry.window.with_raw_handle(|_| ()).is_ok();
            if !live && let Some(effect) = entry.effect.take() {
                effect.destroy();
            }
            live
        });
    }

    fn destroy_all(&mut self) {
        for (_, mut entry) in self.by_window.drain() {
            if let Some(effect) = entry.effect.take() {
                effect.destroy();
            }
        }
        if let Some(manager) = self.manager.take() {
            manager.destroy();
        }
    }
}

/// Phase-1 result of probing compositor blur globals before any round trip.
pub(super) enum InitialBind {
    /// Bound `ext_background_effect_manager_v1`; caller must run one round trip and then call
    /// [`finalize_ext_probe`] to confirm `Capability::Blur` or fall back to KDE blur.
    PendingExt(Manager),
    /// Resolved immediately (`Kde` or `Unavailable`); no round trip needed.
    Resolved(Manager),
}

/// Starts binding a compositor blur manager from `globals`.
pub(super) fn start_bind_manager<D>(
    globals: &GlobalList,
    qh: &QueueHandle<D>,
    has_compositor: bool,
) -> InitialBind
where
    D: Dispatch<ExtBackgroundEffectManagerV1, ()> + Dispatch<OrgKdeKwinBlurManager, ()> + 'static,
{
    if has_compositor
        && let Ok(manager) = globals.bind::<ExtBackgroundEffectManagerV1, _, _>(qh, 1..=1, ())
    {
        return InitialBind::PendingExt(Manager::Ext {
            manager,
            blur_supported: false,
        });
    }
    InitialBind::Resolved(bind_kde_or_unavailable(globals, qh))
}

fn bind_kde_or_unavailable<D>(globals: &GlobalList, qh: &QueueHandle<D>) -> Manager
where
    D: Dispatch<OrgKdeKwinBlurManager, ()> + 'static,
{
    if let Ok(manager) = globals.bind::<OrgKdeKwinBlurManager, _, _>(qh, 1..=1, ()) {
        return Manager::Kde(manager);
    }
    Manager::Unavailable
}

/// Completes the `ext_background_effect_manager_v1` probe after its initial round trip: keeps
/// `Manager::Ext` when `Capability::Blur` was advertised, or destroys the unused ext manager
/// and falls back to `org_kde_kwin_blur_manager` (or `Manager::Unavailable`).
pub(super) fn finalize_ext_probe<D>(
    slot: &mut Option<Manager>,
    roundtrip_ok: bool,
    globals: &GlobalList,
    qh: &QueueHandle<D>,
) -> Protocol
where
    D: Dispatch<OrgKdeKwinBlurManager, ()> + 'static,
{
    if roundtrip_ok
        && matches!(
            slot,
            Some(Manager::Ext {
                blur_supported: true,
                ..
            })
        )
    {
        return Protocol::ExtBackgroundEffect;
    }
    if let Some(old) = slot.take() {
        old.destroy();
    }
    let fallback = if roundtrip_ok {
        bind_kde_or_unavailable(globals, qh)
    } else {
        Manager::Unavailable
    };
    let protocol = fallback.protocol();
    *slot = Some(fallback);
    protocol
}

/// Borrowed surface, compositor, and queue handle for applying a blur toggle.
pub(super) struct SurfaceTarget<'a, D> {
    pub(super) compositor: Option<&'a WlCompositor>,
    pub(super) surface: &'a WlSurface,
    pub(super) qh: &'a QueueHandle<D>,
}

/// Applies `enabled` to `target.surface` using `manager`, reusing or updating `slot` so
/// `ext_background_effect_manager_v1.get_background_effect` is never sent twice for the same
/// surface. Returns `true` when the requested state was applied to the surface, or `false` when
/// skipped (e.g. `Capability::Blur` is currently absent).
pub(super) fn apply_to_surface<D>(
    manager: &Manager,
    target: &SurfaceTarget<'_, D>,
    slot: &mut Option<SurfaceEffect>,
    enabled: bool,
) -> bool
where
    D: Dispatch<ExtBackgroundEffectSurfaceV1, ()>
        + Dispatch<OrgKdeKwinBlur, ()>
        + Dispatch<WlRegion, ()>
        + 'static,
{
    match manager {
        Manager::Ext {
            manager,
            blur_supported,
        } => {
            if enabled {
                *blur_supported && enable_ext(manager, target, slot)
            } else {
                disable_ext(target.surface, slot);
                true
            }
        }
        Manager::Kde(manager) => {
            if enabled {
                enable_kde(manager, target, slot);
            } else {
                disable_kde(manager, target.surface, slot);
            }
            true
        }
        Manager::Unavailable => false,
    }
}

fn enable_ext<D>(
    manager: &ExtBackgroundEffectManagerV1,
    target: &SurfaceTarget<'_, D>,
    slot: &mut Option<SurfaceEffect>,
) -> bool
where
    D: Dispatch<ExtBackgroundEffectSurfaceV1, ()> + Dispatch<WlRegion, ()> + 'static,
{
    let Some(compositor) = target.compositor else {
        return false;
    };
    if !matches!(slot, Some(SurfaceEffect::Ext(_))) {
        if let Some(old) = slot.take() {
            old.destroy();
        }
        let created = manager.get_background_effect(target.surface, target.qh, ());
        *slot = Some(SurfaceEffect::Ext(created));
    }
    let Some(SurfaceEffect::Ext(effect)) = slot.as_ref() else {
        return false;
    };
    let region = compositor.create_region(target.qh, ());
    region.add(0, 0, FULL_SURFACE_EXTENT, FULL_SURFACE_EXTENT);
    effect.set_blur_region(Some(&region));
    region.destroy();
    target.surface.commit();
    true
}

fn disable_ext(surface: &WlSurface, slot: &Option<SurfaceEffect>) {
    if let Some(SurfaceEffect::Ext(effect)) = slot.as_ref() {
        effect.set_blur_region(None);
        surface.commit();
    }
}

fn enable_kde<D>(
    manager: &OrgKdeKwinBlurManager,
    target: &SurfaceTarget<'_, D>,
    slot: &mut Option<SurfaceEffect>,
) where
    D: Dispatch<OrgKdeKwinBlur, ()> + 'static,
{
    if !matches!(slot, Some(SurfaceEffect::Kde(_))) {
        if let Some(old) = slot.take() {
            old.destroy();
        }
        let created = manager.create(target.surface, target.qh, ());
        *slot = Some(SurfaceEffect::Kde(created));
    }
    let Some(SurfaceEffect::Kde(blur)) = slot.as_ref() else {
        return;
    };
    // In `org_kde_kwin_blur`, a NULL region blurs the entire surface behind it.
    blur.set_region(None);
    blur.commit();
    target.surface.commit();
}

fn disable_kde(
    manager: &OrgKdeKwinBlurManager,
    surface: &WlSurface,
    slot: &mut Option<SurfaceEffect>,
) {
    if let Some(effect) = slot.take() {
        manager.unset(surface);
        effect.destroy();
        surface.commit();
    }
}

/// Attaches `window` to the shared Wayland display substrate and binds a blur manager if the
/// compositor advertises one. When `ext_background_effect_manager_v1` is bound for the first
/// time, performs one round trip to observe its initial `capabilities` event before selecting
/// `ExtBackgroundEffect` or falling back to `KdeBlur`.
pub(super) fn attach<S: 'static>(window: &WindowHandle, app: &AppHandle<S>) -> Protocol {
    let handle_app = app.clone();
    let turn: Turn = Arc::new(move |f| {
        handle_app.update_if(move |_, _| {
            f();
            false
        });
    });
    let Ok((transfer, _)) = Transfer::attach(window, turn, Wants::default()) else {
        return Protocol::None;
    };
    let Some(conn) = transfer.shared_conn() else {
        return Protocol::None;
    };
    if conn.is_failed() {
        return Protocol::None;
    }

    let needs_roundtrip = conn.with_globals(|globals, qh, st| {
        let has_compositor = st.compositor().is_some();
        let blur = st.extension::<BlurState>();
        blur.sweep_closed();
        if !blur.hooked {
            blur.hooked = true;
            transfer.before_shutdown(Box::new(teardown));
        }
        if blur.manager.is_some() {
            return Ok(blur.protocol());
        }
        match start_bind_manager(globals, qh, has_compositor) {
            InitialBind::PendingExt(ext) => {
                blur.manager = Some(ext);
                Err(())
            }
            InitialBind::Resolved(manager) => {
                let protocol = manager.protocol();
                blur.manager = Some(manager);
                Ok(protocol)
            }
        }
    });

    let protocol = match needs_roundtrip {
        Some(Ok(protocol)) => protocol,
        Some(Err(())) => {
            let roundtrip_ok = conn.roundtrip().is_ok();
            conn.with_globals(|globals, qh, st| {
                finalize_ext_probe(
                    &mut st.extension::<BlurState>().manager,
                    roundtrip_ok,
                    globals,
                    qh,
                )
            })
            .unwrap_or(Protocol::None)
        }
        None => Protocol::None,
    };
    conn.flush();
    protocol
}

/// Applies `enabled` to `window`'s live `wl_surface` on the UI thread.
/// Returns `true` when the requested state took effect on the surface.
pub(super) fn apply(window: &WindowHandle, enabled: bool) -> bool {
    let Some(transfer) = Transfer::current() else {
        return false;
    };
    let Some(conn) = transfer.shared_conn() else {
        return false;
    };
    if conn.is_failed() {
        return false;
    }
    let applied = conn
        .with_window_surface(window, |surface| {
            let surface_ptr = surface.id().as_ptr() as usize;
            conn.with_globals(|_, qh, st| {
                let compositor = st.compositor().cloned();
                let blur = st.extension::<BlurState>();
                blur.sweep_closed();
                let Some(manager) = blur.manager.as_ref() else {
                    return false;
                };
                let key = window.key();
                if let Some(existing) = blur.by_window.get(key)
                    && existing.surface_ptr != surface_ptr
                    && let Some(mut stale) = blur.by_window.remove(key)
                    && let Some(effect) = stale.effect.take()
                {
                    effect.destroy();
                }
                let entry = blur
                    .by_window
                    .entry(key.to_owned())
                    .or_insert_with(|| SurfaceEntry {
                        window: window.clone(),
                        surface_ptr,
                        desired: enabled,
                        effect: None,
                    });
                entry.desired = enabled;
                let target = SurfaceTarget {
                    compositor: compositor.as_ref(),
                    surface,
                    qh,
                };
                let applied = apply_to_surface(manager, &target, &mut entry.effect, enabled);
                if entry.effect.is_none() && !entry.desired {
                    blur.by_window.remove(key);
                }
                applied
            })
            .unwrap_or(false)
        })
        .unwrap_or(false);
    conn.flush();
    applied
}

/// Releases `window`'s blur object without destroying winit's `wl_surface`.
pub(super) fn detach(window: &WindowHandle) {
    let Some(transfer) = Transfer::current() else {
        return;
    };
    let Some(conn) = transfer.shared_conn() else {
        return;
    };
    if conn.is_failed() {
        return;
    }
    let key = window.key();
    let detached_live = conn.with_window_surface(window, |surface| {
        let _ = conn.with_globals(|_, _, st| {
            let blur = st.extension::<BlurState>();
            let Some(mut entry) = blur.by_window.remove(key) else {
                return;
            };
            let Some(effect) = entry.effect.take() else {
                return;
            };
            match (blur.manager.as_ref(), effect) {
                (Some(Manager::Kde(manager)), SurfaceEffect::Kde(kde)) => {
                    manager.unset(surface);
                    kde.release();
                    surface.commit();
                }
                (_, SurfaceEffect::Ext(ext)) => {
                    ext.destroy();
                    surface.commit();
                }
                (_, other) => other.destroy(),
            }
        });
    });
    if detached_live.is_err() {
        // The native surface is already gone: destroy only our own blur proxy.
        let _ = conn.with_globals(|_, _, st| {
            if let Some(mut entry) = st.extension::<BlurState>().by_window.remove(key)
                && let Some(effect) = entry.effect.take()
            {
                effect.destroy();
            }
        });
    }
    conn.flush();
}

/// Before P0 tears down winit's foreign display backend, destroy all blur proxies on the UI thread.
fn teardown(transfer: &Transfer) {
    if let Some(conn) = transfer.shared_conn() {
        let _ = conn.with_globals(|_, _, st| st.extension::<BlurState>().destroy_all());
        conn.flush();
    }
}

/// Re-applies blur to every live window whose desired state is enabled after `Capability::Blur`
/// transitions from `false` to `true`.
fn reconcile_ext_surfaces(st: &mut State, conn: &Connection, qh: &QueueHandle<State>) {
    let compositor = st.compositor().cloned();
    let blur = st.extension::<BlurState>();
    blur.sweep_closed();
    let Some(Manager::Ext {
        manager,
        blur_supported: true,
    }) = blur.manager.as_ref()
    else {
        return;
    };
    let manager = manager.clone();
    for entry in blur.by_window.values_mut() {
        if !entry.desired {
            continue;
        }
        let _ = with_surface(conn, &entry.window, |surface| {
            let surface_ptr = surface.id().as_ptr() as usize;
            if entry.surface_ptr != surface_ptr {
                if let Some(old) = entry.effect.take() {
                    old.destroy();
                }
                entry.surface_ptr = surface_ptr;
            }
            let target = SurfaceTarget {
                compositor: compositor.as_ref(),
                surface,
                qh,
            };
            let _ = enable_ext(&manager, &target, &mut entry.effect);
        });
    }
    let _ = conn.flush();
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for State {
    fn event(
        st: &mut Self,
        _: &ExtBackgroundEffectManagerV1,
        event: ext_background_effect_manager_v1::Event,
        _: &(),
        conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let ext_background_effect_manager_v1::Event::Capabilities { flags } = event else {
            return;
        };
        let next = capabilities_include_blur(flags);
        let restored = {
            let Some(Manager::Ext { blur_supported, .. }) =
                &mut st.extension::<BlurState>().manager
            else {
                return;
            };
            let was = *blur_supported;
            *blur_supported = next;
            !was && next
        };
        if restored {
            reconcile_ext_surfaces(st, conn, qh);
        }
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ExtBackgroundEffectSurfaceV1,
        _: <ExtBackgroundEffectSurfaceV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<OrgKdeKwinBlurManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &OrgKdeKwinBlurManager,
        _: <OrgKdeKwinBlurManager as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<OrgKdeKwinBlur, ()> for State {
    fn event(
        _: &mut Self,
        _: &OrgKdeKwinBlur,
        _: <OrgKdeKwinBlur as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlRegion, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlRegion,
        _: <WlRegion as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
