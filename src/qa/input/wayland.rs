// SPDX-License-Identifier: MIT

//! Input on a wlroots compositor (the runner's headless sway, `STRATA_QA_DISPLAY=wayland`,
//! added to reproduce input-method bugs such as #1715): a virtual keyboard and
//! pointer on a second connection of this process. The compositor places every toplevel
//! at the output's top-left corner, as the window-manager-less X display does.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io::{Read as _, Write as _},
    os::unix::net::UnixStream,
    path::PathBuf,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use gtk::{gdk, prelude::*};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle,
    globals::{GlobalList, GlobalListContents, registry_queue_init},
    protocol::{wl_pointer, wl_registry, wl_seat::WlSeat},
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
use xkbcommon::xkb;

use super::{Event, Keymap};

/// `KEY_*` and `BTN_*` codes are evdev's, which XKB offsets by 8.
const XKB_EVDEV_OFFSET: u32 = 8;
const KEYMAP_FORMAT_XKB_V1: u32 = 1;
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;
/// One wheel notch, as libinput reports it.
const WHEEL_STEP: f64 = 15.0;
const SWAY_IPC_MAGIC: &[u8; 6] = b"i3-ipc";
const SWAY_RUN_COMMAND: u32 = 0;
const SWAY_SOCKET_VARIABLE: &str = "SWAYSOCK";

struct Globals;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Globals {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(Globals: ignore WlSeat);
wayland_client::delegate_noop!(Globals: ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(Globals: ZwpVirtualKeyboardV1);
wayland_client::delegate_noop!(Globals: ZwlrVirtualPointerManagerV1);
wayland_client::delegate_noop!(Globals: ZwlrVirtualPointerV1);

/// What the compositor's modifier state becomes after a key event.
type Modifiers = [u32; 4];

pub(super) struct Wayland {
    connection: Connection,
    queue: RefCell<EventQueue<Globals>>,
    keyboard: ZwpVirtualKeyboardV1,
    pointer: ZwlrVirtualPointerV1,
    xkb_state: RefCell<xkb::State>,
    modifiers: Cell<Modifiers>,
    position: Cell<(i16, i16)>,
    screen: (u16, u16),
    epoch: Instant,
    delayed: RefCell<Option<JoinHandle<Result<(), String>>>>,
}

impl Wayland {
    /// The keymap is US, compiled here, so key names keep US positions whatever layout a
    /// scenario switches to.
    pub(super) fn connect() -> Result<(Self, Keymap), String> {
        let connection = Connection::connect_to_env()
            .map_err(|error| format!("cannot connect to the Wayland compositor: {error}"))?;
        let (globals, queue) = registry_queue_init::<Globals>(&connection)
            .map_err(|error| format!("cannot list the Wayland globals: {error}"))?;
        let handle = queue.handle();
        let seat: WlSeat = bind(&globals, &handle, 1..=7, "wl_seat")?;
        let keyboards: ZwpVirtualKeyboardManagerV1 =
            bind(&globals, &handle, 1..=1, "zwp_virtual_keyboard_manager_v1")?;
        let pointers: ZwlrVirtualPointerManagerV1 =
            bind(&globals, &handle, 1..=2, "zwlr_virtual_pointer_manager_v1")?;

        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(
            &context,
            "evdev",
            "pc105",
            "us",
            "",
            Some(String::new()),
            xkb::COMPILE_NO_FLAGS,
        )
        .ok_or("cannot compile the US keymap")?;
        let keyboard = keyboards.create_virtual_keyboard(&seat, &handle, ());
        let text = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
        let file = keymap_file(&text)?;
        let size = u32::try_from(text.len() + 1).map_err(|error| error.to_string())?;
        keyboard.keymap(KEYMAP_FORMAT_XKB_V1, file.as_fd_ref(), size);
        let pointer = pointers.create_virtual_pointer(Some(&seat), &handle, ());

        let screen = gdk::Display::default()
            .and_then(|display| display.monitors().item(0))
            .and_downcast::<gdk::Monitor>()
            .map(|monitor| monitor.geometry())
            .ok_or("the Wayland display has no monitor")?;
        let screen = (
            u16::try_from(screen.width()).map_err(|error| error.to_string())?,
            u16::try_from(screen.height()).map_err(|error| error.to_string())?,
        );
        let wayland = Self {
            connection,
            queue: RefCell::new(queue),
            keyboard,
            pointer,
            xkb_state: RefCell::new(xkb::State::new(&keymap)),
            modifiers: Cell::new([0; 4]),
            position: Cell::new((0, 0)),
            screen,
            epoch: Instant::now(),
            delayed: RefCell::new(None),
        };
        wayland.roundtrip()?;
        Ok((wayland, us_keymap(&keymap)))
    }

    fn time(&self) -> u32 {
        timestamp(self.epoch)
    }

    fn roundtrip(&self) -> Result<(), String> {
        self.queue
            .borrow_mut()
            .roundtrip(&mut Globals)
            .map(|_| ())
            .map_err(|error| format!("the Wayland compositor went away: {error}"))
    }

    /// The compositor does not track a virtual keyboard's modifiers, so they follow each key.
    fn modifiers_after(&self, keycode: u8, pressed: bool) -> Option<Modifiers> {
        let mut state = self.xkb_state.borrow_mut();
        state.update_key(
            xkb::Keycode::new(u32::from(keycode)),
            if pressed {
                xkb::KeyDirection::Down
            } else {
                xkb::KeyDirection::Up
            },
        );
        let modifiers = [
            state.serialize_mods(xkb::STATE_MODS_DEPRESSED),
            state.serialize_mods(xkb::STATE_MODS_LATCHED),
            state.serialize_mods(xkb::STATE_MODS_LOCKED),
            state.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE),
        ];
        (self.modifiers.replace(modifiers) != modifiers).then_some(modifiers)
    }

    fn emit(&self, event: Event) -> Result<(), String> {
        let time = self.time();
        match event {
            Event::Key { keycode, pressed } => {
                let modifiers = self.modifiers_after(keycode, pressed);
                press_key(&self.keyboard, time, keycode, pressed, modifiers)?;
            }
            Event::Motion { x, y } => {
                // The compositor keeps the cursor on the output, so record where it can be.
                let clamp = |value: i16, extent: u16| {
                    value.clamp(
                        0,
                        i16::try_from(extent.saturating_sub(1)).unwrap_or(i16::MAX),
                    )
                };
                let (x, y) = (clamp(x, self.screen.0), clamp(y, self.screen.1));
                self.pointer.motion_absolute(
                    time,
                    u32::from(x.unsigned_abs()),
                    u32::from(y.unsigned_abs()),
                    u32::from(self.screen.0),
                    u32::from(self.screen.1),
                );
                self.pointer.frame();
                self.position.set((x, y));
            }
            Event::Button { number, pressed } => self.button(time, number, pressed)?,
        }
        Ok(())
    }

    fn button(&self, time: u32, number: u8, pressed: bool) -> Result<(), String> {
        let scroll = match number {
            4 => Some((wl_pointer::Axis::VerticalScroll, -1)),
            5 => Some((wl_pointer::Axis::VerticalScroll, 1)),
            6 => Some((wl_pointer::Axis::HorizontalScroll, -1)),
            7 => Some((wl_pointer::Axis::HorizontalScroll, 1)),
            _ => None,
        };
        if let Some((axis, steps)) = scroll {
            // X reports a notch as a press and a release; one discrete step stands for both.
            if pressed {
                self.pointer.axis_source(wl_pointer::AxisSource::Wheel);
                self.pointer
                    .axis_discrete(time, axis, WHEEL_STEP * f64::from(steps), steps);
                self.pointer.frame();
            }
            return Ok(());
        }
        let button = match number {
            1 => BTN_LEFT,
            2 => BTN_MIDDLE,
            3 => BTN_RIGHT,
            8 => BTN_SIDE,
            9 => BTN_EXTRA,
            other => return Err(format!("no Wayland button for X button {other}")),
        };
        self.pointer.button(
            time,
            button,
            if pressed {
                wl_pointer::ButtonState::Pressed
            } else {
                wl_pointer::ButtonState::Released
            },
        );
        self.pointer.frame();
        Ok(())
    }

    fn finish_delayed(&self) -> Result<(), String> {
        match self.delayed.borrow_mut().take() {
            Some(thread) => thread
                .join()
                .map_err(|_| "the delayed input thread panicked".to_owned())?,
            None => Ok(()),
        }
    }

    /// The round trip has the compositor deliver the events to GTK's connection before the
    /// main loop is drained.
    pub(super) fn send(&self, event: Event) -> Result<(), String> {
        self.finish_delayed()?;
        self.emit(event)?;
        self.roundtrip()
    }

    pub(super) fn send_after(&self, event: Event, delay: Duration) -> Result<(), String> {
        self.finish_delayed()?;
        let Event::Key { keycode, pressed } = event else {
            std::thread::sleep(delay);
            return self.send(event);
        };
        let modifiers = self.modifiers_after(keycode, pressed);
        let keyboard = self.keyboard.clone();
        let connection = self.connection.clone();
        let epoch = self.epoch;
        let thread = std::thread::Builder::new()
            .name("qa-delayed-key".to_owned())
            .spawn(move || {
                std::thread::sleep(delay);
                press_key(&keyboard, timestamp(epoch), keycode, pressed, modifiers)?;
                connection.flush().map_err(|error| error.to_string())
            })
            .map_err(|error| format!("cannot start the delayed input thread: {error}"))?;
        *self.delayed.borrow_mut() = Some(thread);
        Ok(())
    }

    pub(super) fn sync(&self) -> Result<(), String> {
        self.finish_delayed()?;
        self.roundtrip()
    }

    /// Wayland lets no client read the cursor back, so this is where the last motion sent it,
    /// and a motion the compositor lost goes unnoticed.
    pub(super) fn pointer_position(&self) -> (i16, i16) {
        self.position.get()
    }

    pub(super) fn screen_size(&self) -> (u16, u16) {
        self.screen
    }

    /// Resizes this process's windows through sway, then puts them back in the corner.
    pub(super) fn resize(&self, width: u16, height: u16) -> Result<(), String> {
        let socket = std::env::var_os(SWAY_SOCKET_VARIABLE)
            .map(PathBuf::from)
            .ok_or("resizing on Wayland needs sway's SWAYSOCK")?;
        let pid = std::process::id();
        sway_command(
            &socket,
            &format!(
                "[pid={pid}] resize set width {width} px height {height} px, move position 0 0"
            ),
        )?;
        self.roundtrip()
    }
}

fn bind<I>(
    globals: &GlobalList,
    handle: &QueueHandle<Globals>,
    versions: std::ops::RangeInclusive<u32>,
    name: &str,
) -> Result<I, String>
where
    I: wayland_client::Proxy + 'static,
    Globals: Dispatch<I, ()>,
{
    globals
        .bind(handle, versions, ())
        .map_err(|error| format!("the compositor has no usable {name}: {error}"))
}

fn timestamp(epoch: Instant) -> u32 {
    // Wayland times are milliseconds that wrap.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "event times wrap around at 2^32 ms by definition"
    )]
    let time = epoch.elapsed().as_millis() as u32;
    time
}

fn press_key(
    keyboard: &ZwpVirtualKeyboardV1,
    time: u32,
    keycode: u8,
    pressed: bool,
    modifiers: Option<Modifiers>,
) -> Result<(), String> {
    let key = u32::from(keycode)
        .checked_sub(XKB_EVDEV_OFFSET)
        .ok_or_else(|| format!("keycode {keycode} has no evdev code"))?;
    keyboard.key(time, key, u32::from(pressed));
    if let Some([depressed, latched, locked, group]) = modifiers {
        keyboard.modifiers(depressed, latched, locked, group);
    }
    Ok(())
}

struct KeymapFile(rustix::fd::OwnedFd);

impl KeymapFile {
    fn as_fd_ref(&self) -> rustix::fd::BorrowedFd<'_> {
        use rustix::fd::AsFd as _;
        self.0.as_fd()
    }
}

/// The compositor maps the file, so it holds the keymap and its terminating NUL.
fn keymap_file(text: &str) -> Result<KeymapFile, String> {
    let descriptor = rustix::fs::memfd_create("strata-qa-keymap", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|error| format!("cannot create the keymap file: {error}"))?;
    let mut file = std::fs::File::from(descriptor);
    file.write_all(text.as_bytes())
        .and_then(|()| file.write_all(&[0]))
        .map_err(|error| format!("cannot write the keymap file: {error}"))?;
    Ok(KeymapFile(file.into()))
}

/// The same first-match order as the X keymap: keycodes upward, unshifted before shifted.
fn us_keymap(keymap: &xkb::Keymap) -> Keymap {
    let mut keys = HashMap::new();
    for raw in keymap.min_keycode().raw()..=keymap.max_keycode().raw() {
        let Ok(keycode) = u8::try_from(raw) else {
            break;
        };
        for level in 0..2 {
            if let Some(keysym) = keymap
                .key_get_syms_by_level(xkb::Keycode::new(raw), 0, level)
                .first()
            {
                keys.entry(keysym.raw()).or_insert((keycode, level == 1));
            }
        }
    }
    keys
}

/// A toplevel's window geometry sits at the output's origin; its surface extends past it by
/// the client-side shadow. A popup sits where GDK placed it on its parent.
pub(super) fn surface_origin(surface: &gdk::Surface) -> Result<(i16, i16), String> {
    let narrow = |value: f64| {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "surface offsets are a few pixels"
        )]
        let value = value.round() as i16;
        value
    };
    if let Some(popup) = surface.dynamic_cast_ref::<gdk::Popup>() {
        let parent = popup.parent().ok_or("the popup has no parent surface")?;
        let (x, y) = surface_origin(&parent)?;
        let offset = |value: i32| i16::try_from(value).map_err(|error| error.to_string());
        return Ok((
            x.saturating_add(offset(popup.position_x())?),
            y.saturating_add(offset(popup.position_y())?),
        ));
    }
    let native = gtk::Native::for_surface(surface).ok_or("the surface belongs to no GTK window")?;
    let (shadow_x, shadow_y) = native.surface_transform();
    Ok((-narrow(shadow_x), -narrow(shadow_y)))
}

fn sway_command(socket: &std::path::Path, command: &str) -> Result<(), String> {
    let io = |error: std::io::Error| format!("sway IPC: {error}");
    let mut stream = UnixStream::connect(socket).map_err(io)?;
    let length = u32::try_from(command.len()).map_err(|error| error.to_string())?;
    let mut message = SWAY_IPC_MAGIC.to_vec();
    message.extend(length.to_ne_bytes());
    message.extend(SWAY_RUN_COMMAND.to_ne_bytes());
    message.extend(command.as_bytes());
    stream.write_all(&message).map_err(io)?;
    let mut header = [0; 14];
    stream.read_exact(&mut header).map_err(io)?;
    let mut length = [0; 4];
    length.copy_from_slice(&header[6..10]);
    let mut reply = vec![0; u32::from_ne_bytes(length) as usize];
    stream.read_exact(&mut reply).map_err(io)?;
    let results: Vec<serde_json::Value> =
        serde_json::from_slice(&reply).map_err(|error| format!("sway IPC: {error}"))?;
    match results
        .iter()
        .find(|result| result["success"] != serde_json::Value::Bool(true))
    {
        Some(failure) => Err(format!("sway refused {command:?}: {failure}")),
        None => Ok(()),
    }
}
