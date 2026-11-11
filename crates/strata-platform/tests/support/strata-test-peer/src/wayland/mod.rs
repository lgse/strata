//! The peer on Wayland: one connection, one event queue on the main thread.
//!
//! Commands arrive from the stdin thread through a channel. That thread then sends a
//! `wl_display.sync`, whose reply wakes `blocking_dispatch`, so the loop never polls. Reads and
//! writes of transferred data run on threads of their own, so the peer can serve its own
//! selection while it reads it.

mod data;
mod input;

use std::collections::HashMap;
use std::io::Write;
use std::os::fd::AsFd;
use std::sync::mpsc::{Receiver, channel};

use strata_test_peer::protocol::{
    ActivationToken, Backend, Command, Event, Exported, Mapped, Request, Selection, Via,
};
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer::WlBuffer,
    wl_callback::{self, WlCallback},
    wl_compositor::WlCompositor,
    wl_data_device::WlDataDevice,
    wl_data_device_manager::WlDataDeviceManager,
    wl_keyboard::{self, WlKeyboard},
    wl_pointer::{self, WlPointer},
    wl_registry::WlRegistry,
    wl_seat::{self, WlSeat},
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
    wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::activation::v1::client::{
    xdg_activation_token_v1::{self, XdgActivationTokenV1},
    xdg_activation_v1::XdgActivationV1,
};
use wayland_protocols::xdg::foreign::zv2::client::{
    zxdg_exported_v2::{self, ZxdgExportedV2},
    zxdg_exporter_v2::ZxdgExporterV2,
};
use wayland_protocols::xdg::shell::client::{
    xdg_surface::{self, XdgSurface},
    xdg_toplevel::{self, XdgToplevel},
    xdg_wm_base::{self, XdgWmBase},
};

use crate::{out, read_stdin};
use data::{DataControl, Drop, OfferRef};
use input::Input;

pub fn run() -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("wayland: {e}"))?;
    let (globals, mut queue) =
        registry_queue_init::<St>(&conn).map_err(|e| format!("wayland registry: {e}"))?;
    let qh = queue.handle();
    let mut st = St::new(&conn, &globals, &qh)?;

    let (tx, rx) = channel();
    let display = conn.display();
    let (wake_qh, wake_conn) = (qh.clone(), conn.clone());
    read_stdin(
        move |req| {
            let _ = tx.send(req);
        },
        move || {
            display.sync(&wake_qh, Callback::Wake);
            let _ = wake_conn.flush();
        },
    );
    // The seat's capabilities and data-control's first selection arrive before Ready.
    queue
        .roundtrip(&mut st)
        .map_err(|e| format!("wayland: {e}"))?;
    out::event(Event::Ready {
        backend: Backend::Wayland,
        pid: std::process::id(),
        globals: globals
            .contents()
            .clone_list()
            .into_iter()
            .map(|g| format!("{} v{}", g.interface, g.version))
            .collect(),
        data_control: st.dc.as_ref().map(|d| d.family().to_owned()),
    });
    main_loop(&mut st, &mut queue, &rx)
}

fn main_loop(
    st: &mut St,
    queue: &mut wayland_client::EventQueue<St>,
    rx: &Receiver<Request>,
) -> Result<(), String> {
    loop {
        queue
            .blocking_dispatch(st)
            .map_err(|e| format!("wayland: {e}"))?;
        while let Ok(req) = rx.try_recv() {
            if !st.command(req) {
                let _ = st.conn.flush();
                return Ok(());
            }
        }
        st.settle_pending();
        let _ = st.conn.flush();
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Callback {
    /// The stdin thread's wake-up.
    Wake,
    /// Replies to this request once the server has seen everything before it.
    Reply(u64),
}

struct Window {
    surface: WlSurface,
    xdg: XdgSurface,
    _toplevel: XdgToplevel,
    size: (i32, i32),
    reply: Option<u64>,
}

/// A `selection-set` waiting to see its own offer come back.
struct PendingSet {
    id: u64,
    key: (Via, Selection),
    generation: u64,
    mimes: Vec<String>,
}

pub struct St {
    conn: Connection,
    qh: QueueHandle<St>,
    compositor: WlCompositor,
    shm: WlShm,
    wm_base: XdgWmBase,
    seat: WlSeat,
    ddm: Option<WlDataDeviceManager>,
    data_device: Option<WlDataDevice>,
    dc: Option<DataControl>,
    exporter: Option<ZxdgExporterV2>,
    exported: Vec<ZxdgExportedV2>,
    activation: Option<XdgActivationV1>,
    input: Input,
    pointer: Option<WlPointer>,
    keyboard: Option<WlKeyboard>,
    window: Option<Window>,
    /// The serial of the last keyboard enter on the peer's window, while it has focus.
    focus_serial: Option<u32>,
    /// The serial of the last press on the peer's window, while the button is down.
    press_serial: Option<u32>,
    /// Every live offer's MIME types, data device and data control alike.
    offer_mimes: HashMap<ObjectId, Vec<String>>,
    /// The current selection offer for each protocol and selection, and how many times it
    /// changed.
    current: HashMap<(Via, Selection), OfferRef>,
    generation: HashMap<(Via, Selection), u64>,
    pending_sets: Vec<PendingSet>,
    drop_policy: Option<data::DropPolicy>,
    drop: Option<Drop>,
    buffers: u64,
}

impl St {
    fn new(conn: &Connection, globals: &GlobalList, qh: &QueueHandle<St>) -> Result<St, String> {
        let need = |what: &str| format!("the compositor has no {what}");
        let seat: WlSeat = globals.bind(qh, 1..=7, ()).map_err(|_| need("wl_seat"))?;
        let ddm: Option<WlDataDeviceManager> = globals.bind(qh, 1..=3, ()).ok();
        let data_device = ddm.as_ref().map(|m| m.get_data_device(&seat, qh, ()));
        let dc = DataControl::bind(globals, qh, &seat);
        Ok(St {
            conn: conn.clone(),
            qh: qh.clone(),
            compositor: globals
                .bind(qh, 1..=4, ())
                .map_err(|_| need("wl_compositor"))?,
            shm: globals.bind(qh, 1..=1, ()).map_err(|_| need("wl_shm"))?,
            wm_base: globals
                .bind(qh, 1..=5, ())
                .map_err(|_| need("xdg_wm_base"))?,
            input: Input::bind(globals, qh),
            seat,
            ddm,
            data_device,
            dc,
            exporter: globals.bind(qh, 1..=1, ()).ok(),
            exported: Vec::new(),
            activation: globals.bind(qh, 1..=1, ()).ok(),
            pointer: None,
            keyboard: None,
            window: None,
            focus_serial: None,
            press_serial: None,
            offer_mimes: HashMap::new(),
            current: HashMap::new(),
            generation: HashMap::new(),
            pending_sets: Vec::new(),
            drop_policy: None,
            drop: None,
            buffers: 0,
        })
    }

    /// Runs one request. False once the peer should exit.
    fn command(&mut self, req: Request) -> bool {
        let id = req.id;
        let result = match req.command {
            Command::Quit => {
                out::ok(id);
                return false;
            }
            Command::Sync => {
                self.reply_after_sync(id);
                Ok(())
            }
            Command::Map {
                title,
                width,
                height,
            } => self.map(id, title, width, height),
            Command::Devices {
                pointer,
                keyboard,
                extent,
            } => self
                .input
                .create(&self.qh, &self.seat, pointer, keyboard, extent)
                .map(|()| self.reply_after_sync(id)),
            Command::PointerMove { x, y } => {
                self.input.motion(x, y).map(|()| self.reply_after_sync(id))
            }
            Command::PointerButton { button, pressed } => self
                .input
                .button(button, pressed)
                .map(|()| self.reply_after_sync(id)),
            Command::PointerPath { to, steps, step_ms } => {
                let conn = self.conn.clone();
                self.input
                    .path(to, steps, step_ms, || {
                        let _ = conn.flush();
                    })
                    .map(|()| self.reply_after_sync(id))
            }
            Command::Key { key, pressed } => self
                .input
                .key(&key, pressed)
                .map(|()| self.reply_after_sync(id)),
            Command::Type { text } => self
                .input
                .type_text(&text)
                .map(|()| self.reply_after_sync(id)),
            Command::SelectionSet {
                selection,
                via,
                offers,
            } => self.selection_set(id, selection, via, offers),
            Command::SelectionTargets { selection, via } => {
                let mimes = self.current_mimes(via, selection);
                out::value(id, strata_test_peer::protocol::Targets { mimes });
                Ok(())
            }
            Command::SelectionRead {
                selection,
                via,
                mime,
            } => self.selection_read(id, selection, via, mime),
            Command::DragStart { offers, actions } => self.drag_start(id, offers, actions),
            Command::DropAccept {
                mimes,
                actions,
                preferred,
                read_all,
                ask_answer,
            } => {
                self.drop_policy = (!mimes.is_empty()).then_some(data::DropPolicy {
                    mimes,
                    actions,
                    preferred,
                    read_all,
                    ask_answer,
                });
                out::ok(id);
                Ok(())
            }
            Command::DropProxy => Err("XdndProxy is X11's; a Wayland peer has none".into()),
            Command::Export => self.export(id),
            Command::WindowsOfType { .. } => Err("window types are X11's".into()),
            Command::SelectionReadMultiple { .. } => {
                Err("MULTIPLE is X11's; Wayland reads one type at a time".into())
            }
            Command::ClipboardManager { .. } => {
                Err("CLIPBOARD_MANAGER is X11's; Wayland has no clipboard-manager protocol".into())
            }
            Command::ActivationToken => self.activation_token(id),
        };
        if let Err(e) = result {
            out::reply(id, Err(e));
        }
        true
    }

    fn reply_after_sync(&self, id: u64) {
        self.conn.display().sync(&self.qh, Callback::Reply(id));
    }

    fn map(&mut self, id: u64, title: String, width: u32, height: u32) -> Result<(), String> {
        if self.window.is_some() {
            return Err("the peer's window is already mapped".into());
        }
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg = self.wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg.get_toplevel(&self.qh, ());
        toplevel.set_app_id("strata-test-peer".into());
        toplevel.set_title(title);
        surface.commit();
        self.window = Some(Window {
            surface,
            xdg,
            _toplevel: toplevel,
            size: (width as i32, height as i32),
            reply: Some(id),
        });
        Ok(())
    }

    /// Fills the window with grey at its configured size.
    fn draw(&mut self) {
        let Some(w) = self.window.as_mut() else {
            return;
        };
        let (width, height) = (w.size.0.max(1), w.size.1.max(1));
        let len = (width * height * 4) as usize;
        self.buffers += 1;
        let path = out::scratch_path(&format!("buffer-{}", self.buffers));
        let file = std::fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .and_then(|mut f| f.write_all(&vec![0x80; len]).map(|()| f));
        let _ = std::fs::remove_file(&path);
        let file = match file {
            Ok(f) => f,
            Err(e) => return out::error(format!("window buffer: {e}")),
        };
        let pool = self.shm.create_pool(file.as_fd(), len as i32, &self.qh, ());
        let buffer = pool.create_buffer(
            0,
            width,
            height,
            width * 4,
            wl_shm::Format::Xrgb8888,
            &self.qh,
            (),
        );
        pool.destroy();
        w.surface.attach(Some(&buffer), 0, 0);
        w.surface.damage_buffer(0, 0, width, height);
        w.surface.commit();
        if let Some(id) = w.reply.take() {
            out::value(id, Mapped { xid: None });
        }
    }

    fn export(&mut self, id: u64) -> Result<(), String> {
        let exporter = self
            .exporter
            .as_ref()
            .ok_or("the compositor has no zxdg_exporter_v2")?;
        let window = self.window.as_ref().ok_or("map the peer's window first")?;
        let exported = exporter.export_toplevel(&window.surface, &self.qh, id);
        self.exported.push(exported);
        Ok(())
    }

    /// Asks for an activation token as a launcher does: with the serial of the input that
    /// triggered the launch, the seat and the peer's focused surface. The reply comes with
    /// the token's `done` event.
    fn activation_token(&mut self, id: u64) -> Result<(), String> {
        let activation = self
            .activation
            .as_ref()
            .ok_or("the compositor has no xdg_activation_v1")?;
        let surface = self.surface().ok_or("map the peer's window first")?;
        let serial = self
            .press_serial
            .or(self.focus_serial)
            .ok_or("the peer's window has neither keyboard focus nor a press")?;
        let token = activation.get_activation_token(&self.qh, id);
        token.set_serial(serial, &self.seat);
        token.set_surface(surface);
        token.commit();
        Ok(())
    }

    /// Replies to each `selection-set` whose offer the peer now sees as current.
    fn settle_pending(&mut self) {
        let mut sets = std::mem::take(&mut self.pending_sets);
        sets.retain(|p| {
            let generation = self.generation.get(&p.key).copied().unwrap_or(0);
            let seen = generation > p.generation
                && self.current_mimes(p.key.0, p.key.1).as_ref() == Some(&p.mimes);
            if seen {
                out::ok(p.id);
            }
            !seen
        });
        self.pending_sets = sets;
    }

    fn surface(&self) -> Option<&WlSurface> {
        self.window.as_ref().map(|w| &w.surface)
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for St {
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

delegate_noop!(St: ignore WlCompositor);
delegate_noop!(St: ignore WlShm);
delegate_noop!(St: ignore WlShmPool);
delegate_noop!(St: ignore WlSurface);
delegate_noop!(St: ignore WlDataDeviceManager);
delegate_noop!(St: ignore ZxdgExporterV2);
delegate_noop!(St: ignore XdgActivationV1);

/// Each draw makes a buffer; the compositor releases it once a newer one replaces it.
impl Dispatch<WlBuffer, ()> for St {
    fn event(
        _: &mut Self,
        buffer: &WlBuffer,
        event: <WlBuffer as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wayland_client::protocol::wl_buffer::Event::Release = event {
            buffer.destroy();
        }
    }
}

impl Dispatch<WlCallback, Callback> for St {
    fn event(
        _: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        data: &Callback,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let (wl_callback::Event::Done { .. }, Callback::Reply(id)) = (event, data) {
            out::ok(*id);
        }
    }
}

impl Dispatch<WlSeat, ()> for St {
    fn event(
        st: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        // A headless seat gains its pointer and keyboard when a peer creates virtual ones.
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Pointer) && st.pointer.is_none() {
                st.pointer = Some(seat.get_pointer(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Keyboard) && st.keyboard.is_none() {
                st.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

impl Dispatch<WlPointer, ()> for St {
    fn event(
        st: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Pointer focus goes to stderr: it explains a press that went elsewhere.
        match event {
            wl_pointer::Event::Enter {
                surface_x,
                surface_y,
                ..
            } => out::event(Event::PointerEnter {
                x: surface_x,
                y: surface_y,
            }),
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => out::event(Event::PointerMotion {
                x: surface_x,
                y: surface_y,
            }),
            wl_pointer::Event::Leave { .. } => out::event(Event::PointerLeave),
            wl_pointer::Event::Button {
                serial,
                state: WEnum::Value(state),
                ..
            } => {
                let pressed = state == wl_pointer::ButtonState::Pressed;
                st.press_serial = pressed.then_some(serial);
                out::event(Event::PointerButton { pressed });
            }
            _ => {}
        }
    }
}

impl Dispatch<WlKeyboard, ()> for St {
    fn event(
        st: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter {
                serial, surface, ..
            } if st.surface() == Some(&surface) => {
                st.focus_serial = Some(serial);
                out::event(Event::Focus { focused: true });
            }
            wl_keyboard::Event::Leave { surface, .. } if st.surface() == Some(&surface) => {
                st.focus_serial = None;
                out::event(Event::Focus { focused: false });
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgWmBase, ()> for St {
    fn event(
        _: &mut Self,
        base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for St {
    fn event(
        st: &mut Self,
        xdg: &XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg.ack_configure(serial);
            if st.window.as_ref().is_some_and(|w| &w.xdg == xdg) {
                st.draw();
            }
        }
    }
}

impl Dispatch<XdgToplevel, ()> for St {
    fn event(
        st: &mut Self,
        _: &XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event
            && width > 0
            && height > 0
            && let Some(w) = st.window.as_mut()
        {
            w.size = (width, height);
        }
    }
}

impl Dispatch<XdgActivationTokenV1, u64> for St {
    fn event(
        _: &mut Self,
        token: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        id: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token: value } = event {
            token.destroy();
            out::value(*id, ActivationToken { token: value });
        }
    }
}

impl Dispatch<ZxdgExportedV2, u64> for St {
    fn event(
        _: &mut Self,
        _: &ZxdgExportedV2,
        event: zxdg_exported_v2::Event,
        id: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_exported_v2::Event::Handle { handle } = event {
            out::value(
                *id,
                Exported {
                    handle: format!("wayland:{handle}"),
                },
            );
        }
    }
}
