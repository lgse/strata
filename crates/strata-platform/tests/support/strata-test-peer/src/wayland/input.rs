//! A virtual pointer (`zwlr_virtual_pointer_v1`) and keyboard (`zwp_virtual_keyboard_v1`).
//!
//! A headless sway started with `WLR_LIBINPUT_NO_DEVICES=1` has a seat with no devices, so no
//! client gets pointer events or keyboard focus until a virtual device exists. sway routes
//! these devices to Xwayland too, so they drive X11 clients as well.

use std::io::Write;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use strata_test_peer::protocol::{Button, chord_keys};
use wayland_client::globals::GlobalList;
use wayland_client::protocol::{wl_pointer, wl_seat::WlSeat};
use wayland_client::{QueueHandle, delegate_noop};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
use xkbcommon::xkb;

use super::St;
use crate::out;

/// Absolute motion is sent in hundredths of a point, for sub-point positions.
const SUBPOINT: f64 = 100.0;

pub struct Input {
    pointer_manager: Option<ZwlrVirtualPointerManagerV1>,
    keyboard_manager: Option<ZwpVirtualKeyboardManagerV1>,
    pointer: Option<ZwlrVirtualPointerV1>,
    keyboard: Option<Keyboard>,
    extent: [u32; 2],
    position: (f64, f64),
    start: Instant,
}

struct Keyboard {
    device: ZwpVirtualKeyboardV1,
    keymap: xkb::Keymap,
    state: xkb::State,
}

impl Input {
    pub fn bind(globals: &GlobalList, qh: &QueueHandle<St>) -> Self {
        Input {
            pointer_manager: globals.bind(qh, 1..=2, ()).ok(),
            keyboard_manager: globals.bind(qh, 1..=1, ()).ok(),
            pointer: None,
            keyboard: None,
            extent: [1, 1],
            position: (0.0, 0.0),
            start: Instant::now(),
        }
    }

    pub fn create(
        &mut self,
        qh: &QueueHandle<St>,
        seat: &WlSeat,
        pointer: bool,
        keyboard: bool,
        extent: [u32; 2],
    ) -> Result<(), String> {
        self.extent = [extent[0].max(1), extent[1].max(1)];
        // The keyboard first: clients need it for focus, and the pointer for their first enter.
        if keyboard && self.keyboard.is_none() {
            let manager = self
                .keyboard_manager
                .as_ref()
                .ok_or("the compositor has no zwp_virtual_keyboard_manager_v1")?;
            let device = manager.create_virtual_keyboard(seat, qh, ());
            let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
            let keymap = xkb::Keymap::new_from_names(
                &ctx,
                "",
                "",
                "us",
                "",
                None,
                xkb::KEYMAP_COMPILE_NO_FLAGS,
            )
            .ok_or("xkbcommon could not compile a US keymap (is xkb-data installed?)")?;
            let mut text = keymap
                .get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
                .into_bytes();
            text.push(0);
            let path = out::scratch_path("keymap.xkb");
            let file = std::fs::File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .and_then(|mut f| f.write_all(&text).map(|()| f))
                .map_err(|e| format!("keymap file: {e}"))?;
            let _ = std::fs::remove_file(&path);
            // 1 is WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1.
            device.keymap(1, file.as_fd(), text.len() as u32);
            let state = xkb::State::new(&keymap);
            self.keyboard = Some(Keyboard {
                device,
                keymap,
                state,
            });
        }
        if pointer && self.pointer.is_none() {
            let manager = self
                .pointer_manager
                .as_ref()
                .ok_or("the compositor has no zwlr_virtual_pointer_manager_v1")?;
            self.pointer = Some(manager.create_virtual_pointer(Some(seat), qh, ()));
        }
        Ok(())
    }

    fn time(&self) -> u32 {
        // Wayland input times are milliseconds that wrap.
        (self.start.elapsed().as_millis() % u128::from(u32::MAX)) as u32
    }

    fn pointer(&self) -> Result<&ZwlrVirtualPointerV1, String> {
        self.pointer
            .as_ref()
            .ok_or_else(|| "send devices with pointer: true first".to_owned())
    }

    pub fn motion(&mut self, x: f64, y: f64) -> Result<(), String> {
        let p = self.pointer()?;
        let [w, h] = self.extent;
        let clamp = |v: f64, max: u32| (v.clamp(0.0, f64::from(max)) * SUBPOINT) as u32;
        p.motion_absolute(
            self.time(),
            clamp(x, w),
            clamp(y, h),
            (f64::from(w) * SUBPOINT) as u32,
            (f64::from(h) * SUBPOINT) as u32,
        );
        p.frame();
        self.position = (x, y);
        Ok(())
    }

    pub fn button(&mut self, button: Button, pressed: bool) -> Result<(), String> {
        let p = self.pointer()?;
        let state = if pressed {
            wl_pointer::ButtonState::Pressed
        } else {
            wl_pointer::ButtonState::Released
        };
        p.button(self.time(), button.code(), state);
        p.frame();
        Ok(())
    }

    /// Moves in `steps` straight steps, flushing and pausing after each, so the compositor and
    /// clients see a drag move rather than jump.
    pub fn path(
        &mut self,
        to: [f64; 2],
        steps: u32,
        step_ms: u64,
        flush: impl Fn(),
    ) -> Result<(), String> {
        let (x0, y0) = self.position;
        let steps = steps.max(1);
        for i in 1..=steps {
            let t = f64::from(i) / f64::from(steps);
            self.motion(x0 + (to[0] - x0) * t, y0 + (to[1] - y0) * t)?;
            flush();
            std::thread::sleep(Duration::from_millis(step_ms));
        }
        Ok(())
    }

    fn keyboard(&mut self) -> Result<&mut Keyboard, String> {
        self.keyboard
            .as_mut()
            .ok_or_else(|| "send devices with keyboard: true first".to_owned())
    }

    /// A chord ([`chord_keys`]), or one edge of it.
    pub fn key(&mut self, chord: &str, pressed: Option<bool>) -> Result<(), String> {
        let time = self.time();
        let kb = self.keyboard()?;
        let mut codes = Vec::new();
        for name in chord_keys(chord)? {
            let (code, level) = kb.find_name(&name)?;
            if level > 0 && !codes.contains(&kb.shift()?) {
                codes.push(kb.shift()?);
            }
            codes.push(code);
        }
        if pressed != Some(false) {
            codes.iter().for_each(|c| kb.press(time, *c, true));
        }
        if pressed != Some(true) {
            codes.iter().rev().for_each(|c| kb.press(time, *c, false));
        }
        Ok(())
    }

    pub fn type_text(&mut self, text: &str) -> Result<(), String> {
        let time = self.time();
        let kb = self.keyboard()?;
        let shift = kb.shift()?;
        for ch in text.chars() {
            let sym = xkb::utf32_to_keysym(u32::from(ch));
            let (code, level) = kb
                .find(sym)
                .ok_or_else(|| format!("{ch:?} is not on the US layout"))?;
            if level > 0 {
                kb.press(time, shift, true);
            }
            kb.press(time, code, true);
            kb.press(time, code, false);
            if level > 0 {
                kb.press(time, shift, false);
            }
        }
        Ok(())
    }
}

impl Keyboard {
    /// The key and shift level (0 or 1) that types `sym` in layout 0: level 0 anywhere first.
    fn find(&self, sym: xkb::Keysym) -> Option<(xkb::Keycode, u32)> {
        for level in 0..2 {
            let mut found = None;
            self.keymap.key_for_each(|keymap, code| {
                if found.is_none() && keymap.key_get_syms_by_level(code, 0, level).contains(&sym) {
                    found = Some(code);
                }
            });
            if let Some(code) = found {
                return Some((code, level));
            }
        }
        None
    }

    fn find_name(&self, name: &str) -> Result<(xkb::Keycode, u32), String> {
        let mut sym = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
        if sym.raw() == 0 {
            sym = xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE);
        }
        if sym.raw() == 0 {
            return Err(format!("no keysym named {name:?}"));
        }
        self.find(sym)
            .ok_or_else(|| format!("{name:?} is not on the US layout"))
    }

    fn shift(&self) -> Result<xkb::Keycode, String> {
        self.find_name("Shift_L").map(|(code, _)| code)
    }

    /// One key edge, then the modifier state it leaves, as a real keyboard reports it.
    fn press(&mut self, time: u32, code: xkb::Keycode, down: bool) {
        // Wayland key codes are evdev codes: XKB's minus 8.
        self.device
            .key(time, code.raw().saturating_sub(8), u32::from(down));
        let direction = if down {
            xkb::KeyDirection::Down
        } else {
            xkb::KeyDirection::Up
        };
        self.state.update_key(code, direction);
        self.device.modifiers(
            self.state.serialize_mods(xkb::STATE_MODS_DEPRESSED),
            self.state.serialize_mods(xkb::STATE_MODS_LATCHED),
            self.state.serialize_mods(xkb::STATE_MODS_LOCKED),
            self.state.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE),
        );
    }
}

delegate_noop!(St: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(St: ignore ZwlrVirtualPointerV1);
delegate_noop!(St: ignore ZwpVirtualKeyboardManagerV1);
delegate_noop!(St: ignore ZwpVirtualKeyboardV1);
