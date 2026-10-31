// SPDX-License-Identifier: MIT

//! With `STRATA_INPUT_LOG` set, each event is appended in the format `.agents/skills/qa-scenarios/reference.md` defines.

mod wayland;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fs::File,
    io::Write as _,
    time::Instant,
};

use gtk::{gdk, glib::translate::IntoGlib, prelude::*};
use x11rb::{
    connection::{Connection, RequestConnection},
    protocol::{
        xproto::{self, ConnectionExt as _},
        xtest,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use super::protocol::Modifier;

/// Each keysym's keycode, and whether it needs Shift.
type Keymap = HashMap<u32, (u8, bool)>;

pub(crate) struct Injector {
    backend: Backend,
    keymap: Keymap,
    log: Option<RefCell<File>>,
    log_origin: Cell<(i16, i16)>,
    last_release: Cell<Option<Instant>>,
}

pub(crate) const INPUT_LOG_VARIABLE: &str = "STRATA_INPUT_LOG";

#[derive(Clone, Copy, Debug)]
pub(crate) enum Event {
    Key { keycode: u8, pressed: bool },
    Button { number: u8, pressed: bool },
    Motion { x: i16, y: i16 },
}

enum Backend {
    X11(Box<X11>),
    Wayland(Box<wayland::Wayland>),
}

struct X11 {
    connection: RustConnection,
    root: xproto::Window,
}

impl Injector {
    /// XTEST on an X display; a virtual keyboard and pointer on a Wayland compositor.
    pub(crate) fn connect() -> Result<Self, String> {
        let wayland = gdk::Display::default()
            .is_some_and(|display| display.is::<gdk4_wayland::WaylandDisplay>());
        let (backend, keymap) = if wayland {
            let (backend, keymap) = wayland::Wayland::connect()?;
            (Backend::Wayland(Box::new(backend)), keymap)
        } else {
            let (backend, keymap) = X11::connect()?;
            (Backend::X11(Box::new(backend)), keymap)
        };
        let log = match std::env::var_os(INPUT_LOG_VARIABLE).filter(|path| !path.is_empty()) {
            Some(path) => Some(RefCell::new(
                File::options()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .map_err(|error| format!("cannot open {path:?} for the input log: {error}"))?,
            )),
            None => None,
        };
        Ok(Self {
            backend,
            keymap,
            log,
            log_origin: Cell::new((0, 0)),
            last_release: Cell::new(None),
        })
    }

    pub(crate) fn logging(&self) -> bool {
        self.log.is_some()
    }

    pub(crate) fn set_log_origin(&self, origin: (i16, i16)) {
        self.log_origin.set(origin);
    }

    pub(crate) fn last_release(&self) -> Option<Instant> {
        self.last_release.get()
    }

    pub(crate) fn record_release(&self) {
        self.last_release.set(Some(Instant::now()));
    }

    pub(crate) fn keycode(&self, keysym: u32) -> Result<(u8, bool), String> {
        self.keymap
            .get(&keysym)
            .copied()
            .ok_or_else(|| format!("keysym {keysym:#06x} is not on the keyboard layout"))
    }

    pub(crate) fn send(&self, event: Event) -> Result<(), String> {
        match &self.backend {
            Backend::X11(x11) => {
                x11.fake(event, 0)?;
                // The round trip guarantees the server has generated the events for
                // GTK's connection before the main loop is drained.
                x11.connection.sync().map_err(|error| error.to_string())?;
            }
            Backend::Wayland(wayland) => wayland.send(event)?,
        }
        self.log(event)
    }

    /// Timed outside the main loop, so neither this process nor its main loop can hold the
    /// event back.
    pub(crate) fn send_after(
        &self,
        event: Event,
        delay: std::time::Duration,
    ) -> Result<(), String> {
        match &self.backend {
            Backend::X11(x11) => {
                let delay = u32::try_from(delay.as_millis()).map_err(|error| error.to_string())?;
                x11.fake(event, delay)?;
                x11.connection.flush().map_err(|error| error.to_string())?;
            }
            Backend::Wayland(wayland) => wayland.send_after(event, delay)?,
        }
        self.log(event)
    }

    pub(crate) fn sync(&self) -> Result<(), String> {
        match &self.backend {
            Backend::X11(x11) => x11.connection.sync().map_err(|error| error.to_string()),
            Backend::Wayland(wayland) => wayland.sync(),
        }
    }

    fn log(&self, event: Event) -> Result<(), String> {
        let Some(log) = self.log.as_ref() else {
            return Ok(());
        };
        let (origin_x, origin_y) = self.log_origin.get();
        let state = |pressed: bool| if pressed { "down" } else { "up" };
        let line = match event {
            Event::Key { keycode, pressed } => format!("key {keycode} {}", state(pressed)),
            Event::Motion { x, y } => format!("motion {} {}", x - origin_x, y - origin_y),
            Event::Button { number, pressed } => {
                let (x, y) = self.pointer_position()?;
                format!(
                    "pointer {} {} button {number} {}",
                    x - origin_x,
                    y - origin_y,
                    state(pressed)
                )
            }
        };
        writeln!(log.borrow_mut(), "{line}")
            .map_err(|error| format!("cannot write the input log: {error}"))
    }

    pub(crate) fn pointer_position(&self) -> Result<(i16, i16), String> {
        match &self.backend {
            Backend::X11(x11) => x11.pointer_position(),
            Backend::Wayland(wayland) => Ok(wayland.pointer_position()),
        }
    }

    pub(crate) fn surface_origin(&self, surface: &gdk::Surface) -> Result<(i16, i16), String> {
        match &self.backend {
            Backend::X11(x11) => x11.surface_origin(surface),
            Backend::Wayland(_) => wayland::surface_origin(surface),
        }
    }

    pub(crate) fn screen_size(&self) -> Result<(u16, u16), String> {
        match &self.backend {
            Backend::X11(x11) => x11.screen_size(),
            Backend::Wayland(wayland) => Ok(wayland.screen_size()),
        }
    }

    pub(crate) fn grab(
        &self,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
    ) -> Result<Vec<[u8; 3]>, String> {
        match &self.backend {
            Backend::X11(x11) => x11.grab(x, y, width, height),
            Backend::Wayland(_) => {
                Err("reading screen pixels needs the X11 display, not Wayland".to_owned())
            }
        }
    }

    pub(crate) fn resize(
        &self,
        surface: &gdk::Surface,
        width: u16,
        height: u16,
    ) -> Result<(), String> {
        match &self.backend {
            Backend::X11(x11) => x11.resize(surface, width, height),
            Backend::Wayland(wayland) => wayland.resize(width, height),
        }
    }
}

impl X11 {
    fn connect() -> Result<(Self, Keymap), String> {
        let (connection, screen) =
            x11rb::connect(None).map_err(|error| format!("cannot open the X display: {error}"))?;
        if connection
            .extension_information(xtest::X11_EXTENSION_NAME)
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Err("the X server has no XTEST extension".to_owned());
        }
        let root = connection.setup().roots[screen].root;
        // Without this, synthetic events are discarded while the application
        // holds a pointer grab, which is the state a drag runs in.
        xtest::grab_control(&connection, true).map_err(|error| error.to_string())?;
        let keymap = read_keymap(&connection)?;
        Ok((Self { connection, root }, keymap))
    }

    fn fake(&self, event: Event, delay: u32) -> Result<(), String> {
        let (kind, detail, x, y) = match event {
            Event::Key { keycode, pressed } => (
                if pressed {
                    xproto::KEY_PRESS_EVENT
                } else {
                    xproto::KEY_RELEASE_EVENT
                },
                keycode,
                0,
                0,
            ),
            Event::Button { number, pressed } => (
                if pressed {
                    xproto::BUTTON_PRESS_EVENT
                } else {
                    xproto::BUTTON_RELEASE_EVENT
                },
                number,
                0,
                0,
            ),
            Event::Motion { x, y } => (xproto::MOTION_NOTIFY_EVENT, 0, x, y),
        };
        xtest::fake_input(&self.connection, kind, detail, delay, self.root, x, y, 0)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn pointer_position(&self) -> Result<(i16, i16), String> {
        let reply = self
            .connection
            .query_pointer(self.root)
            .map_err(|error| error.to_string())?
            .reply()
            .map_err(|error| error.to_string())?;
        Ok((reply.root_x, reply.root_y))
    }

    fn surface_origin(&self, surface: &gdk::Surface) -> Result<(i16, i16), String> {
        let reply = self
            .connection
            .translate_coordinates(xid(surface)?, self.root, 0, 0)
            .map_err(|error| error.to_string())?
            .reply()
            .map_err(|error| error.to_string())?;
        Ok((reply.dst_x, reply.dst_y))
    }

    fn screen_size(&self) -> Result<(u16, u16), String> {
        self.connection
            .setup()
            .roots
            .iter()
            .find(|screen| screen.root == self.root)
            .map(|screen| (screen.width_in_pixels, screen.height_in_pixels))
            .ok_or_else(|| "the X screen is unknown".to_owned())
    }

    fn grab(&self, x: i16, y: i16, width: u16, height: u16) -> Result<Vec<[u8; 3]>, String> {
        let reply = self
            .connection
            .get_image(
                xproto::ImageFormat::Z_PIXMAP,
                self.root,
                x,
                y,
                width,
                height,
                u32::MAX,
            )
            .map_err(|error| error.to_string())?
            .reply()
            .map_err(|error| {
                format!("cannot read the screen at {x},{y} {width}x{height}: {error}")
            })?;
        let setup = self.connection.setup();
        let visual = setup
            .roots
            .iter()
            .flat_map(|screen| &screen.allowed_depths)
            .flat_map(|depth| &depth.visuals)
            .find(|visual| visual.visual_id == reply.visual)
            .ok_or("the screen's visual is unknown")?;
        let format = setup
            .pixmap_formats
            .iter()
            .find(|format| format.depth == reply.depth)
            .ok_or("the screen's pixel format is unknown")?;
        let bytes = match format.bits_per_pixel {
            24 => 3,
            32 => 4,
            other => return Err(format!("cannot read {other}-bit pixels")),
        };
        let pad = usize::from(format.scanline_pad) / 8;
        let stride = (usize::from(width) * bytes).div_ceil(pad) * pad;
        let little = setup.image_byte_order == xproto::ImageOrder::LSB_FIRST;
        let channel = |pixel: u32, mask: u32| -> u8 {
            if mask == 0 {
                return 0;
            }
            let value = (pixel & mask) >> mask.trailing_zeros();
            let maximum = mask >> mask.trailing_zeros();
            u8::try_from(value * 255 / maximum).unwrap_or(u8::MAX)
        };
        let mut pixels = Vec::with_capacity(usize::from(width) * usize::from(height));
        for row in 0..usize::from(height) {
            for column in 0..usize::from(width) {
                let start = row * stride + column * bytes;
                let raw = reply
                    .data
                    .get(start..start + bytes)
                    .ok_or("the screen image is shorter than its size")?;
                let pixel = raw.iter().enumerate().fold(0u32, |pixel, (index, byte)| {
                    let shift = if little { index } else { bytes - 1 - index };
                    pixel | u32::from(*byte) << (8 * shift)
                });
                pixels.push([
                    channel(pixel, visual.red_mask),
                    channel(pixel, visual.green_mask),
                    channel(pixel, visual.blue_mask),
                ]);
            }
        }
        Ok(pixels)
    }

    fn resize(&self, surface: &gdk::Surface, width: u16, height: u16) -> Result<(), String> {
        self.connection
            .configure_window(
                xid(surface)?,
                &xproto::ConfigureWindowAux::new()
                    .width(u32::from(width))
                    .height(u32::from(height)),
            )
            .map_err(|error| error.to_string())?;
        self.connection.sync().map_err(|error| error.to_string())
    }
}

fn xid(surface: &gdk::Surface) -> Result<xproto::Window, String> {
    let surface = surface
        .downcast_ref::<gdk4_x11::X11Surface>()
        .ok_or("the window is not on an X11 surface")?;
    u32::try_from(surface.xid()).map_err(|_| "the X window id does not fit 32 bits".to_owned())
}

/// Read once at startup, before a scenario switches layouts, so key names keep US positions.
fn read_keymap(connection: &RustConnection) -> Result<Keymap, String> {
    let setup = connection.setup();
    let (first, last) = (setup.min_keycode, setup.max_keycode);
    let mapping = connection
        .get_keyboard_mapping(first, last - first + 1)
        .map_err(|error| error.to_string())?
        .reply()
        .map_err(|error| error.to_string())?;
    let width = usize::from(mapping.keysyms_per_keycode);
    let mut keymap = HashMap::new();
    if width == 0 {
        return Err("the X server returned no keyboard mapping".to_owned());
    }
    for (index, keysyms) in mapping.keysyms.chunks(width).enumerate() {
        let Ok(offset) = u8::try_from(index) else {
            break;
        };
        for (level, keysym) in keysyms.iter().take(2).enumerate() {
            if *keysym != 0 {
                keymap
                    .entry(*keysym)
                    .or_insert((first.saturating_add(offset), level == 1));
            }
        }
    }
    Ok(keymap)
}

pub(crate) fn keysym(name: &str) -> Result<u32, String> {
    let mut characters = name.chars();
    if let (Some(character), None) = (characters.next(), characters.next()) {
        let code = u32::from(character);
        return Ok(if code < 0x100 {
            code
        } else {
            0x0100_0000 | code
        });
    }
    gdk::Key::from_name(name)
        .map(IntoGlib::into_glib)
        .ok_or_else(|| format!("no keysym named {name:?}"))
}

pub(crate) fn modifier_keysym(modifier: Modifier) -> Result<u32, String> {
    keysym(modifier.keysym_name())
}
