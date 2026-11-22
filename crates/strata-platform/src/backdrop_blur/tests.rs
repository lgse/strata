// SPDX-License-Identifier: MIT

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use wayland_backend::client::Backend;
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::{self, Capability, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};
use wayland_protocols_plasma::blur::client::{
    org_kde_kwin_blur::OrgKdeKwinBlur, org_kde_kwin_blur_manager::OrgKdeKwinBlurManager,
};

use super::wayland::{
    FULL_SURFACE_EXTENT, InitialBind, Manager, SurfaceEffect, SurfaceTarget, apply_to_surface,
    capabilities_include_blur, finalize_ext_probe, start_bind_manager,
};
use super::{BackdropBlur, Protocol};

/// Decoded Wayland wire message: `(sender_id, opcode, payload_words)`.
#[derive(Debug, PartialEq, Eq)]
struct WireMsg {
    object: u32,
    opcode: u16,
    words: Vec<u32>,
}

fn write_msg(stream: &mut UnixStream, object: u32, opcode: u16, words: &[u32]) {
    let byte_len = u16::try_from(8 + words.len() * 4).expect("small test message");
    let header = [object, (u32::from(byte_len) << 16) | u32::from(opcode)];
    for word in header.into_iter().chain(words.iter().copied()) {
        stream.write_all(&word.to_ne_bytes()).expect("write word");
    }
}

fn push_wl_string(words: &mut Vec<u32>, text: &str) {
    let bytes = text.as_bytes();
    let len_with_nul = u32::try_from(bytes.len() + 1).expect("short interface name");
    words.push(len_with_nul);
    let mut padded = Vec::with_capacity((bytes.len() + 4) & !3);
    padded.extend_from_slice(bytes);
    padded.push(0);
    while !padded.len().is_multiple_of(4) {
        padded.push(0);
    }
    for &chunk in padded.as_chunks::<4>().0 {
        words.push(u32::from_ne_bytes(chunk));
    }
}

fn read_msg(stream: &mut UnixStream) -> Option<WireMsg> {
    let mut hdr = [0u8; 8];
    if stream.read_exact(&mut hdr).is_err() {
        return None;
    }
    let object = u32::from_ne_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
    let size_opcode = u32::from_ne_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
    let byte_len = (size_opcode >> 16) as usize;
    let opcode = (size_opcode & 0xffff) as u16;
    assert!(
        byte_len >= 8 && byte_len.is_multiple_of(4),
        "valid wire length"
    );
    let payload_bytes = byte_len - 8;
    let mut raw = vec![0u8; payload_bytes];
    stream.read_exact(&mut raw).expect("read payload");
    let words = raw
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .map(u32::from_ne_bytes)
        .collect();
    Some(WireMsg {
        object,
        opcode,
        words,
    })
}

struct MockCompositor {
    conn: Connection,
    caps_tx: mpsc::Sender<u32>,
    server: thread::JoinHandle<Vec<WireMsg>>,
}

impl MockCompositor {
    /// Queues a `capabilities(flags)` event to be emitted on the next `roundtrip`.
    fn queue_capabilities(&self, flags: u32) {
        self.caps_tx.send(flags).expect("queue capabilities");
    }

    fn finish(self) -> Vec<WireMsg> {
        drop(self.caps_tx);
        drop(self.conn);
        self.server.join().expect("server thread")
    }
}

/// Minimal Wayland server that answers `wl_display.get_registry` and `wl_display.sync`,
/// advertises `globals`, emits initial `ext_background_effect_manager_v1.capabilities(initial_caps)`
/// when that global is bound, drains any queued dynamic capability updates on each `sync`, and
/// records all client requests until disconnect.
fn spawn_mock_compositor(
    globals: Vec<(&'static str, u32)>,
    initial_ext_caps: Option<u32>,
) -> MockCompositor {
    let (client_sock, mut server_sock) = UnixStream::pair().expect("socket pair");
    server_sock
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let (caps_tx, caps_rx) = mpsc::channel::<u32>();
    let ext_global_name = globals
        .iter()
        .position(|(iface, _)| *iface == "ext_background_effect_manager_v1")
        .map(|idx| u32::try_from(idx + 1).expect("global name"));

    let server = thread::spawn(move || {
        let mut recorded = Vec::new();
        let mut registry_id = 0u32;
        let mut ext_manager_id = None::<u32>;
        while let Some(msg) = read_msg(&mut server_sock) {
            if msg.object == 1 && msg.opcode == 1 {
                // wl_display.get_registry(new_id)
                registry_id = msg.words[0];
                for (idx, (iface, version)) in globals.iter().enumerate() {
                    let mut words = vec![u32::try_from(idx + 1).expect("global name")];
                    push_wl_string(&mut words, iface);
                    words.push(*version);
                    write_msg(&mut server_sock, registry_id, 0, &words);
                }
            } else if msg.object == 1 && msg.opcode == 0 {
                // wl_display.sync(new_id): flush any queued dynamic capability events first.
                if let Some(mgr_id) = ext_manager_id {
                    for flags in caps_rx.try_iter() {
                        write_msg(&mut server_sock, mgr_id, 0, &[flags]);
                    }
                }
                let callback_id = msg.words[0];
                write_msg(&mut server_sock, callback_id, 0, &[1]);
                write_msg(&mut server_sock, 1, 1, &[callback_id]);
            } else {
                if registry_id != 0
                    && msg.object == registry_id
                    && msg.opcode == 0
                    && ext_global_name == msg.words.first().copied()
                    && let Some(&new_id) = msg.words.last()
                {
                    ext_manager_id = Some(new_id);
                    if let Some(flags) = initial_ext_caps {
                        write_msg(&mut server_sock, new_id, 0, &[flags]);
                    }
                }
                recorded.push(msg);
            }
        }
        recorded
    });
    let backend = Backend::connect(client_sock).expect("connect mock compositor");
    MockCompositor {
        conn: Connection::from_backend(backend),
        caps_tx,
        server,
    }
}

#[derive(Default)]
struct MockState {
    manager: Option<Manager>,
    /// Counts how many times `capabilities` flipped `blur_supported` from `false` to `true`.
    restored_count: usize,
}

/// Runs the same two-phase bind + roundtrip + finalize sequence that `wayland::attach` runs.
fn probe_manager(
    globals: &GlobalList,
    queue: &mut EventQueue<MockState>,
    state: &mut MockState,
    has_compositor: bool,
) -> Protocol {
    let qh = queue.handle();
    match start_bind_manager(globals, &qh, has_compositor) {
        InitialBind::PendingExt(ext) => {
            state.manager = Some(ext);
            let rt_ok = queue.roundtrip(state).is_ok();
            finalize_ext_probe(&mut state.manager, rt_ok, globals, &qh)
        }
        InitialBind::Resolved(manager) => {
            let protocol = manager.protocol();
            state.manager = Some(manager);
            protocol
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for MockState {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlCompositor, ()> for MockState {
    fn event(
        _: &mut Self,
        _: &WlCompositor,
        _: <WlCompositor as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSurface, ()> for MockState {
    fn event(
        _: &mut Self,
        _: &WlSurface,
        _: <WlSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlRegion, ()> for MockState {
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

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for MockState {
    fn event(
        st: &mut Self,
        _: &ExtBackgroundEffectManagerV1,
        event: ext_background_effect_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_background_effect_manager_v1::Event::Capabilities { flags } = event
            && let Some(Manager::Ext { blur_supported, .. }) = &mut st.manager
        {
            let next = capabilities_include_blur(flags);
            if !*blur_supported && next {
                st.restored_count += 1;
            }
            *blur_supported = next;
        }
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, ()> for MockState {
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

impl Dispatch<OrgKdeKwinBlurManager, ()> for MockState {
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

impl Dispatch<OrgKdeKwinBlur, ()> for MockState {
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

#[test]
fn unattached_handle_tracks_desired_state_idempotently_and_noops_safely() {
    let blur = BackdropBlur::new();
    let cloned = blur.clone();

    assert!(!blur.is_enabled());
    assert!(!blur.is_attached());
    assert_eq!(blur.protocol(), Protocol::None);

    blur.set_enabled(true);
    blur.set_enabled(true);
    assert!(blur.is_enabled());
    assert!(cloned.is_enabled());
    assert!(!cloned.is_attached());

    cloned.set_enabled(false);
    cloned.set_enabled(false);
    assert!(!blur.is_enabled());

    blur.detach();
    assert!(!blur.is_attached());
    assert_eq!(blur.protocol(), Protocol::None);
}

#[test]
fn capabilities_bitfield_handles_known_and_unknown_future_bits() {
    assert!(!capabilities_include_blur(
        WEnum::Value(Capability::empty())
    ));
    assert!(capabilities_include_blur(WEnum::Value(Capability::Blur)));
    assert!(!capabilities_include_blur(WEnum::Unknown(0b10)));
    assert!(capabilities_include_blur(WEnum::Unknown(0b11)));
}

#[test]
fn protocol_selection_prefers_ext_background_effect_then_kde_blur_then_none() {
    // Both advertised with Blur capability -> ext_background_effect_v1 wins.
    let mock = spawn_mock_compositor(
        vec![
            ("wl_compositor", 4),
            ("org_kde_kwin_blur_manager", 1),
            ("ext_background_effect_manager_v1", 1),
        ],
        Some(1),
    );
    let (globals, mut queue) = registry_queue_init::<MockState>(&mock.conn).expect("registry init");
    let mut state = MockState::default();
    assert_eq!(
        probe_manager(&globals, &mut queue, &mut state, true),
        Protocol::ExtBackgroundEffect
    );
    if let Some(manager) = state.manager.take() {
        manager.destroy();
    }
    drop((globals, queue));
    let _ = mock.finish();

    // Only KWin blur advertised -> falls back to KdeBlur.
    let mock = spawn_mock_compositor(
        vec![("wl_compositor", 4), ("org_kde_kwin_blur_manager", 1)],
        None,
    );
    let (globals, mut queue) = registry_queue_init::<MockState>(&mock.conn).expect("registry init");
    let mut state = MockState::default();
    assert_eq!(
        probe_manager(&globals, &mut queue, &mut state, true),
        Protocol::KdeBlur
    );
    if let Some(manager) = state.manager.take() {
        manager.destroy();
    }
    drop((globals, queue));
    let _ = mock.finish();

    // Neither advertised (Sway / GNOME) -> silent None.
    let mock = spawn_mock_compositor(vec![("wl_compositor", 4)], None);
    let (globals, mut queue) = registry_queue_init::<MockState>(&mock.conn).expect("registry init");
    let mut state = MockState::default();
    assert_eq!(
        probe_manager(&globals, &mut queue, &mut state, true),
        Protocol::None
    );
    drop((globals, queue));
    let _ = mock.finish();
}

#[test]
fn ext_without_blur_capability_destroys_ext_manager_and_falls_back_to_kde_blur() {
    let mock = spawn_mock_compositor(
        vec![
            ("wl_compositor", 4),
            ("ext_background_effect_manager_v1", 1),
            ("org_kde_kwin_blur_manager", 1),
        ],
        Some(0),
    );
    let (globals, mut queue) = registry_queue_init::<MockState>(&mock.conn).expect("registry init");
    let qh = queue.handle();
    let compositor = globals
        .bind::<WlCompositor, _, _>(&qh, 1..=4, ())
        .expect("wl_compositor");
    let surface = compositor.create_surface(&qh, ());

    let mut state = MockState::default();
    let InitialBind::PendingExt(ext) = start_bind_manager(&globals, &qh, true) else {
        panic!("expected pending ext bind");
    };
    let ext_manager_id = match &ext {
        Manager::Ext { manager, .. } => manager.id().protocol_id(),
        _ => panic!("expected Manager::Ext"),
    };
    state.manager = Some(ext);
    let rt_ok = queue.roundtrip(&mut state).is_ok();
    let protocol = finalize_ext_probe(&mut state.manager, rt_ok, &globals, &qh);
    assert_eq!(protocol, Protocol::KdeBlur);

    let manager = state.manager.take().expect("kde manager");
    let Manager::Kde(ref kde_mgr) = manager else {
        panic!("expected fallback to org_kde_kwin_blur_manager");
    };
    let kde_mgr_id = kde_mgr.id().protocol_id();
    let surface_id = surface.id().protocol_id();

    let target = SurfaceTarget {
        compositor: Some(&compositor),
        surface: &surface,
        qh: &qh,
    };
    let mut slot: Option<SurfaceEffect> = None;
    assert!(apply_to_surface(&manager, &target, &mut slot, true));
    let blur_id = match slot.as_ref() {
        Some(SurfaceEffect::Kde(blur)) => blur.id().protocol_id(),
        _ => panic!("expected kde blur object"),
    };
    assert!(apply_to_surface(&manager, &target, &mut slot, false));

    manager.destroy();
    mock.conn.flush().expect("flush");
    drop((surface, compositor, globals, queue));
    let msgs = mock.finish();

    // Unused `ext_background_effect_manager_v1` was destroyed (opcode 0).
    assert!(
        msgs.iter()
            .any(|m| m.object == ext_manager_id && m.opcode == 0),
        "expected unused ext_background_effect_manager_v1.destroy"
    );
    // Never called `get_background_effect` on ext manager.
    assert!(
        !msgs
            .iter()
            .any(|m| m.object == ext_manager_id && m.opcode == 1)
    );
    // Fallback used `org_kde_kwin_blur_manager.create` (opcode 0) and `unset` (opcode 1).
    assert!(
        msgs.iter().any(|m| m.object == kde_mgr_id
            && m.opcode == 0
            && m.words == vec![blur_id, surface_id])
    );
    assert!(
        msgs.iter()
            .any(|m| m.object == kde_mgr_id && m.opcode == 1 && m.words == vec![surface_id])
    );
}

#[test]
fn ext_with_blur_and_unknown_future_capability_bit_enables_blur() {
    // 0b11 = Capability::Blur (1) | unknown future capability bit (2) -> WEnum::Unknown(3).
    let mock = spawn_mock_compositor(
        vec![
            ("wl_compositor", 4),
            ("ext_background_effect_manager_v1", 1),
        ],
        Some(0b11),
    );
    let (globals, mut queue) = registry_queue_init::<MockState>(&mock.conn).expect("registry init");
    let qh = queue.handle();
    let compositor = globals
        .bind::<WlCompositor, _, _>(&qh, 1..=4, ())
        .expect("wl_compositor");
    let surface = compositor.create_surface(&qh, ());

    let mut state = MockState::default();
    assert_eq!(
        probe_manager(&globals, &mut queue, &mut state, true),
        Protocol::ExtBackgroundEffect
    );
    let manager = state.manager.take().expect("ext manager");
    let Manager::Ext {
        manager: ref ext_mgr,
        blur_supported,
    } = manager
    else {
        panic!("expected Manager::Ext");
    };
    assert!(blur_supported);
    let ext_mgr_id = ext_mgr.id().protocol_id();
    let surface_id = surface.id().protocol_id();

    let target = SurfaceTarget {
        compositor: Some(&compositor),
        surface: &surface,
        qh: &qh,
    };
    let mut slot: Option<SurfaceEffect> = None;
    assert!(apply_to_surface(&manager, &target, &mut slot, true));
    let effect_id = match slot.as_ref() {
        Some(SurfaceEffect::Ext(effect)) => effect.id().protocol_id(),
        _ => panic!("expected ext surface effect"),
    };

    if let Some(effect) = slot.take() {
        effect.destroy();
    }
    manager.destroy();
    mock.conn.flush().expect("flush");
    drop((surface, compositor, globals, queue));
    let msgs = mock.finish();

    assert!(msgs.iter().any(|m| m.object == ext_mgr_id
        && m.opcode == 1
        && m.words == vec![effect_id, surface_id]));
    assert!(
        msgs.iter()
            .any(|m| m.object == effect_id && m.opcode == 1 && m.words[0] != 0)
    );
    assert!(msgs.iter().any(|m| m.object == surface_id && m.opcode == 6));
}

#[test]
fn ext_background_effect_reuses_surface_object_and_toggles_blur_region() {
    let mock = spawn_mock_compositor(
        vec![
            ("wl_compositor", 4),
            ("ext_background_effect_manager_v1", 1),
        ],
        Some(1),
    );
    let (globals, mut queue) = registry_queue_init::<MockState>(&mock.conn).expect("registry init");
    let qh = queue.handle();
    let compositor = globals
        .bind::<WlCompositor, _, _>(&qh, 1..=4, ())
        .expect("wl_compositor");
    let surface = compositor.create_surface(&qh, ());

    let mut state = MockState::default();
    assert_eq!(
        probe_manager(&globals, &mut queue, &mut state, true),
        Protocol::ExtBackgroundEffect
    );
    let manager = state.manager.take().expect("ext manager");
    let Manager::Ext {
        manager: ref ext_mgr,
        ..
    } = manager
    else {
        panic!("expected ext_background_effect_manager_v1");
    };
    let manager_id = ext_mgr.id().protocol_id();
    let surface_id = surface.id().protocol_id();

    let target = SurfaceTarget {
        compositor: Some(&compositor),
        surface: &surface,
        qh: &qh,
    };
    let mut slot: Option<SurfaceEffect> = None;

    // First enable creates the effect object, sets a full-surface blur region, and commits.
    assert!(apply_to_surface(&manager, &target, &mut slot, true));
    let effect_id = match slot.as_ref() {
        Some(SurfaceEffect::Ext(effect)) => effect.id().protocol_id(),
        _ => panic!("expected ext surface effect"),
    };
    // Re-enabling on the same surface reuses the existing effect object (no duplicate create).
    assert!(apply_to_surface(&manager, &target, &mut slot, true));
    // Disabling clears the blur region (NULL object id 0) and commits.
    assert!(apply_to_surface(&manager, &target, &mut slot, false));
    // Explicit destroy releases the effect object cleanly without destroying wl_surface.
    if let Some(effect) = slot.take() {
        effect.destroy();
    }
    manager.destroy();
    mock.conn.flush().expect("flush");
    drop((surface, compositor, globals, queue));

    let msgs = mock.finish();

    // Exactly one `get_background_effect` (opcode 1 on manager).
    let get_calls: Vec<_> = msgs
        .iter()
        .filter(|m| m.object == manager_id && m.opcode == 1)
        .collect();
    assert_eq!(get_calls.len(), 1);
    assert_eq!(get_calls[0].words, vec![effect_id, surface_id]);

    // `set_blur_region` (opcode 1 on effect): two non-null region ids, then one NULL (0).
    let set_region_calls: Vec<_> = msgs
        .iter()
        .filter(|m| m.object == effect_id && m.opcode == 1)
        .collect();
    assert_eq!(set_region_calls.len(), 3);
    assert_ne!(set_region_calls[0].words[0], 0);
    assert_ne!(set_region_calls[1].words[0], 0);
    assert_eq!(set_region_calls[2].words[0], 0);

    // Region `add` (opcode 1 on wl_region) covers 0, 0, FULL_SURFACE_EXTENT, FULL_SURFACE_EXTENT.
    let first_region_id = set_region_calls[0].words[0];
    let add_msg = msgs
        .iter()
        .find(|m| m.object == first_region_id && m.opcode == 1)
        .expect("wl_region.add");
    assert_eq!(
        add_msg.words,
        vec![0, 0, FULL_SURFACE_EXTENT as u32, FULL_SURFACE_EXTENT as u32]
    );
    // Region was destroyed immediately after `set_blur_region` (opcode 0 on wl_region).
    assert!(
        msgs.iter()
            .any(|m| m.object == first_region_id && m.opcode == 0)
    );

    // Surface committed three times (enable, re-enable, disable) and never destroyed by blur.
    let commits = msgs
        .iter()
        .filter(|m| m.object == surface_id && m.opcode == 6)
        .count();
    assert_eq!(commits, 3);
    let surface_destroys = msgs
        .iter()
        .filter(|m| m.object == surface_id && m.opcode == 0)
        .count();
    assert_eq!(surface_destroys, 0);
}

#[test]
fn dynamic_capabilities_transition_gates_applied_result_and_recovers_on_restore() {
    let mock = spawn_mock_compositor(
        vec![
            ("wl_compositor", 4),
            ("ext_background_effect_manager_v1", 1),
        ],
        Some(1),
    );
    let (globals, mut queue) = registry_queue_init::<MockState>(&mock.conn).expect("registry init");
    let qh = queue.handle();
    let compositor = globals
        .bind::<WlCompositor, _, _>(&qh, 1..=4, ())
        .expect("wl_compositor");
    let surface = compositor.create_surface(&qh, ());

    let mut state = MockState::default();
    assert_eq!(
        probe_manager(&globals, &mut queue, &mut state, true),
        Protocol::ExtBackgroundEffect
    );
    let manager_id = match state.manager.as_ref() {
        Some(Manager::Ext { manager, .. }) => manager.id().protocol_id(),
        _ => panic!("expected Manager::Ext"),
    };
    let target = SurfaceTarget {
        compositor: Some(&compositor),
        surface: &surface,
        qh: &qh,
    };
    let mut slot: Option<SurfaceEffect> = None;

    // 1. Blur initially supported -> apply(true) succeeds.
    assert!(apply_to_surface(
        state.manager.as_ref().expect("manager"),
        &target,
        &mut slot,
        true
    ));
    let effect_id = match slot.as_ref() {
        Some(SurfaceEffect::Ext(effect)) => effect.id().protocol_id(),
        _ => panic!("expected ext surface effect"),
    };

    // 2. Compositor dynamically withdraws Blur (`capabilities(0)`):
    mock.queue_capabilities(0);
    queue.roundtrip(&mut state).expect("capabilities(0)");
    // Disable still succeeds (clears region); re-enable returns false so HandleState::applied
    // is NOT prematurely marked Some(true).
    assert!(apply_to_surface(
        state.manager.as_ref().expect("manager"),
        &target,
        &mut slot,
        false
    ));
    assert!(!apply_to_surface(
        state.manager.as_ref().expect("manager"),
        &target,
        &mut slot,
        true
    ));

    // 3. Compositor restores Blur with an extra unknown bit (`capabilities(0b11)`):
    let before_restores = state.restored_count;
    mock.queue_capabilities(0b11);
    queue.roundtrip(&mut state).expect("capabilities(0b11)");
    assert_eq!(state.restored_count, before_restores + 1);
    // Now enable succeeds again and reuses the existing effect proxy without recreating it.
    assert!(apply_to_surface(
        state.manager.as_ref().expect("manager"),
        &target,
        &mut slot,
        true
    ));

    if let Some(effect) = slot.take() {
        effect.destroy();
    }
    if let Some(manager) = state.manager.take() {
        manager.destroy();
    }
    mock.conn.flush().expect("flush");
    drop((surface, compositor, globals, queue));
    let msgs = mock.finish();

    // Still exactly 1 `get_background_effect` across the entire lifecycle.
    let get_calls = msgs
        .iter()
        .filter(|m| m.object == manager_id && m.opcode == 1)
        .count();
    assert_eq!(get_calls, 1);

    // `set_blur_region`: initial enable (non-null), disable (null), restored enable (non-null);
    // the skipped enable while capabilities == 0 sent NO wire message!
    let regions: Vec<u32> = msgs
        .iter()
        .filter(|m| m.object == effect_id && m.opcode == 1)
        .map(|m| m.words[0])
        .collect();
    assert_eq!(regions.len(), 3);
    assert_ne!(regions[0], 0);
    assert_eq!(regions[1], 0);
    assert_ne!(regions[2], 0);
}

#[test]
fn kde_blur_fallback_creates_commits_and_unsets_cleanly() {
    let mock = spawn_mock_compositor(
        vec![("wl_compositor", 4), ("org_kde_kwin_blur_manager", 1)],
        None,
    );
    let (globals, mut queue) = registry_queue_init::<MockState>(&mock.conn).expect("registry init");
    let qh = queue.handle();
    let compositor = globals
        .bind::<WlCompositor, _, _>(&qh, 1..=4, ())
        .expect("wl_compositor");
    let surface = compositor.create_surface(&qh, ());

    let mut state = MockState::default();
    assert_eq!(
        probe_manager(&globals, &mut queue, &mut state, true),
        Protocol::KdeBlur
    );
    let manager = state.manager.take().expect("kde manager");
    let Manager::Kde(ref kde_mgr) = manager else {
        panic!("expected org_kde_kwin_blur_manager");
    };
    let manager_id = kde_mgr.id().protocol_id();
    let surface_id = surface.id().protocol_id();

    let target = SurfaceTarget {
        compositor: Some(&compositor),
        surface: &surface,
        qh: &qh,
    };
    let mut slot: Option<SurfaceEffect> = None;

    assert!(apply_to_surface(&manager, &target, &mut slot, true));
    let blur_id = match slot.as_ref() {
        Some(SurfaceEffect::Kde(blur)) => blur.id().protocol_id(),
        _ => panic!("expected kde blur object"),
    };
    assert!(apply_to_surface(&manager, &target, &mut slot, false));
    assert!(slot.is_none());

    manager.destroy();
    mock.conn.flush().expect("flush");
    drop((surface, compositor, globals, queue));

    let msgs = mock.finish();

    // `org_kde_kwin_blur_manager.create` (opcode 0)
    assert!(
        msgs.iter().any(|m| m.object == manager_id
            && m.opcode == 0
            && m.words == vec![blur_id, surface_id])
    );
    // `org_kde_kwin_blur.set_region(None)` (opcode 1, region = 0) and `commit` (opcode 0)
    assert!(
        msgs.iter()
            .any(|m| m.object == blur_id && m.opcode == 1 && m.words == vec![0])
    );
    assert!(msgs.iter().any(|m| m.object == blur_id && m.opcode == 0));
    // `org_kde_kwin_blur_manager.unset(surface)` (opcode 1) and `blur.release` (opcode 2)
    assert!(
        msgs.iter()
            .any(|m| m.object == manager_id && m.opcode == 1 && m.words == vec![surface_id])
    );
    assert!(msgs.iter().any(|m| m.object == blur_id && m.opcode == 2));
    // Surface committed twice (enable + disable) and never destroyed.
    let commits = msgs
        .iter()
        .filter(|m| m.object == surface_id && m.opcode == 6)
        .count();
    assert_eq!(commits, 2);
}
