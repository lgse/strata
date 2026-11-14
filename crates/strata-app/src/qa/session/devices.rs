// SPDX-License-Identifier: MIT

//! The session host's keyboard and pointer: a Wayland virtual keyboard and pointer on a
//! connection of the session thread's own, so input reaches the window through the
//! compositor and winit as a user's does. Modelled on the GTK harness's
//! (`src/qa/input/wayland.rs`). The keymap is US, compiled here, as the scenarios name keys
//! by where they are on a US keyboard.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io::Write as _,
    time::Instant,
};

use rustix::fd::{AsFd as _, OwnedFd};
use strata_qa::Modifier;
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

/// XKB numbers a key by its evdev code plus 8.
const XKB_EVDEV_OFFSET: u32 = 8;
const KEYMAP_FORMAT_XKB_V1: u32 = 1;
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
/// One wheel notch, as libinput reports it.
const WHEEL_STEP: f64 = 15.0;

/// A key of the US keyboard: its XKB keycode, and whether it types the keysym with Shift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct UsKey {
    pub(super) keycode: u32,
    pub(super) shift: bool,
}

/// The XKB keycode of the left-hand key of each modifier, and of right Alt for AltGr.
pub(super) fn modifier_key(modifier: Modifier) -> u32 {
    let evdev = match modifier {
        Modifier::Ctrl => 29,
        Modifier::Shift => 42,
        Modifier::Alt => 56,
        Modifier::AltGr => 100,
    };
    evdev + XKB_EVDEV_OFFSET
}

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

/// What the compositor's modifier state becomes after a key event: depressed, latched,
/// locked and the layout.
type Modifiers = [u32; 4];

pub(super) struct Devices {
    queue: RefCell<EventQueue<Globals>>,
    keyboard: ZwpVirtualKeyboardV1,
    pointer: ZwlrVirtualPointerV1,
    state: RefCell<xkb::State>,
    modifiers: Cell<Modifiers>,
    /// The first key of the keymap that types each keysym, unshifted before shifted.
    keys: HashMap<u32, UsKey>,
    position: Cell<(i16, i16)>,
    /// The output's size, which absolute motions are in.
    screen: (u32, u32),
    epoch: Instant,
}

impl Devices {
    pub(super) fn connect(screen: (u32, u32)) -> Result<Self, String> {
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
        keyboard.keymap(KEYMAP_FORMAT_XKB_V1, file.as_fd(), size);
        let pointer = pointers.create_virtual_pointer(Some(&seat), &handle, ());
        let devices = Self {
            queue: RefCell::new(queue),
            keyboard,
            pointer,
            state: RefCell::new(xkb::State::new(&keymap)),
            modifiers: Cell::new([0; 4]),
            keys: us_keys(&keymap),
            position: Cell::new((0, 0)),
            screen,
            epoch: Instant::now(),
        };
        devices.sync()?;
        Ok(devices)
    }

    /// The key a scenario names (GDK's names, which are XKB's keysym names), or the key that
    /// types a one-character name.
    pub(super) fn named(&self, name: &str) -> Result<UsKey, String> {
        let mut keysym = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
        if keysym.raw() == 0 {
            keysym = xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE);
        }
        let mut characters = name.chars();
        if let (Some(character), None) = (characters.next(), characters.next()) {
            keysym = xkb::utf32_to_keysym(u32::from(character));
        }
        self.keys
            .get(&keysym.raw())
            .copied()
            .ok_or_else(|| format!("no key of a US keyboard is named {name:?}"))
    }

    /// The key that types `character` on the US keyboard.
    pub(super) fn typing(&self, character: char) -> Result<UsKey, String> {
        let keysym = xkb::utf32_to_keysym(u32::from(character));
        self.keys
            .get(&keysym.raw())
            .copied()
            .ok_or_else(|| format!("the session host's US keyboard has no key for {character:?}"))
    }

    fn time(&self) -> u32 {
        // Wayland's times are milliseconds that wrap.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "event times wrap around at 2^32 ms by definition"
        )]
        let time = self.epoch.elapsed().as_millis() as u32;
        time
    }

    /// Has the compositor take every request sent so far, and send on the events it makes.
    pub(super) fn sync(&self) -> Result<(), String> {
        self.queue
            .borrow_mut()
            .roundtrip(&mut Globals)
            .map(|_| ())
            .map_err(|error| format!("the Wayland compositor went away: {error}"))
    }

    /// The compositor does not track a virtual keyboard's modifiers, so they follow each key.
    fn modifiers_after(&self, keycode: u32, pressed: bool) -> Option<Modifiers> {
        let mut state = self.state.borrow_mut();
        let direction = if pressed {
            xkb::KeyDirection::Down
        } else {
            xkb::KeyDirection::Up
        };
        state.update_key(xkb::Keycode::new(keycode), direction);
        let modifiers = [
            state.serialize_mods(xkb::STATE_MODS_DEPRESSED),
            state.serialize_mods(xkb::STATE_MODS_LATCHED),
            state.serialize_mods(xkb::STATE_MODS_LOCKED),
            state.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE),
        ];
        (self.modifiers.replace(modifiers) != modifiers).then_some(modifiers)
    }

    pub(super) fn key(&self, keycode: u32, pressed: bool) -> Result<(), String> {
        let modifiers = self.modifiers_after(keycode, pressed);
        let evdev = keycode
            .checked_sub(XKB_EVDEV_OFFSET)
            .ok_or_else(|| format!("keycode {keycode} has no evdev code"))?;
        self.keyboard.key(self.time(), evdev, u32::from(pressed));
        if let Some([depressed, latched, locked, group]) = modifiers {
            self.keyboard.modifiers(depressed, latched, locked, group);
        }
        self.sync()
    }

    /// Moves the pointer to (x, y) on the output, which keeps it inside.
    pub(super) fn motion(&self, x: i16, y: i16) -> Result<(), String> {
        let clamp = |value: i16, extent: u32| {
            let last = i16::try_from(extent.saturating_sub(1)).unwrap_or(i16::MAX);
            value.clamp(0, last)
        };
        let (x, y) = (clamp(x, self.screen.0), clamp(y, self.screen.1));
        self.pointer.motion_absolute(
            self.time(),
            u32::from(x.unsigned_abs()),
            u32::from(y.unsigned_abs()),
            self.screen.0,
            self.screen.1,
        );
        self.pointer.frame();
        self.position.set((x, y));
        self.sync()
    }

    /// Wayland lets no client read the cursor back, so this is where the last motion put it.
    pub(super) fn position(&self) -> (i16, i16) {
        self.position.get()
    }

    /// A button as X numbers them: 4 and 5 are one notch of the wheel up and down, sent on
    /// the press.
    pub(super) fn button(&self, number: u8, pressed: bool) -> Result<(), String> {
        let time = self.time();
        let notch = match number {
            4 => Some(-1),
            5 => Some(1),
            _ => None,
        };
        if let Some(steps) = notch {
            if pressed {
                let axis = wl_pointer::Axis::VerticalScroll;
                self.pointer.axis_source(wl_pointer::AxisSource::Wheel);
                self.pointer
                    .axis_discrete(time, axis, WHEEL_STEP * f64::from(steps), steps);
                self.pointer.frame();
            }
            return self.sync();
        }
        let button = match number {
            1 => BTN_LEFT,
            2 => BTN_MIDDLE,
            3 => BTN_RIGHT,
            other => return Err(format!("the pointer has no button {other}")),
        };
        let state = if pressed {
            wl_pointer::ButtonState::Pressed
        } else {
            wl_pointer::ButtonState::Released
        };
        self.pointer.button(time, button, state);
        self.pointer.frame();
        self.sync()
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

/// The compositor maps the file, so it holds the keymap and its terminating NUL.
fn keymap_file(text: &str) -> Result<OwnedFd, String> {
    let descriptor =
        rustix::fs::memfd_create("strata-rune-qa-keymap", rustix::fs::MemfdFlags::CLOEXEC)
            .map_err(|error| format!("cannot create the keymap file: {error}"))?;
    let mut file = std::fs::File::from(descriptor);
    file.write_all(text.as_bytes())
        .and_then(|()| file.write_all(&[0]))
        .map_err(|error| format!("cannot write the keymap file: {error}"))?;
    Ok(file.into())
}

/// The keymap's keys by the keysym each types: keycodes upward, unshifted before shifted, as
/// the GTK harness picks them.
fn us_keys(keymap: &xkb::Keymap) -> HashMap<u32, UsKey> {
    let mut keys = HashMap::new();
    for keycode in keymap.min_keycode().raw()..=keymap.max_keycode().raw() {
        for level in 0..2 {
            let syms = keymap.key_get_syms_by_level(xkb::Keycode::new(keycode), 0, level);
            if let Some(keysym) = syms.first() {
                keys.entry(keysym.raw()).or_insert(UsKey {
                    keycode,
                    shift: level == 1,
                });
            }
        }
    }
    keys
}
