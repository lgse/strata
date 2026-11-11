// SPDX-License-Identifier: MIT

//! GTK's key input: X keycodes for a chord, the Shift a keysym needs, and a borrowed
//! keycode for a character the layout lacks.

use std::time::{Duration, Instant};

use gtk::{glib, glib::translate::IntoGlib, prelude::*};

use super::{
    EVENT_GAP, POLL_INTERVAL,
    backend::Gtk,
    input::{self, Event},
    protocol::{self, Chord, Modifier},
    settle,
};

impl Gtk {
    /// Lets `window` paint a frame, then waits out what is left of `gap`.
    async fn pause(&self, window: &gtk::Widget, gap: Duration) {
        let started = Instant::now();
        settle(window.downcast_ref::<gtk::Window>()).await;
        if let Some(remaining) = gap.checked_sub(started.elapsed()) {
            glib::timeout_future(remaining).await;
        }
    }

    async fn emit(&self, window: &gtk::Widget, event: Event, gap: Duration) -> Result<(), String> {
        self.injector()?.send(event)?;
        self.pause(window, gap).await;
        Ok(())
    }

    async fn send_key(
        &self,
        window: &gtk::Widget,
        keycode: u8,
        pressed: bool,
    ) -> Result<(), String> {
        self.emit(window, Event::Key { keycode, pressed }, EVENT_GAP)
            .await
    }

    /// The keycode of a key or modifier named as a `key_down` step names it.
    pub(super) fn named_keycode(&self, name: &str) -> Result<u8, String> {
        let keysym = match Modifier::parse(name) {
            Ok(modifier) => input::modifier_keysym(modifier)?,
            Err(_) => input::keysym(name)?,
        };
        let (keycode, _) = self.injector()?.keycode(keysym)?;
        Ok(keycode)
    }

    pub(super) fn modifier_keycodes(&self, modifiers: &[Modifier]) -> Result<Vec<u8>, String> {
        let injector = self.injector()?;
        modifiers
            .iter()
            .map(|modifier| {
                let (keycode, _) = injector.keycode(input::modifier_keysym(*modifier)?)?;
                Ok(keycode)
            })
            .collect()
    }

    pub(super) async fn hold_keys(
        &self,
        window: &gtk::Widget,
        modifiers: &[Modifier],
    ) -> Result<Vec<u8>, String> {
        let keycodes = self.modifier_keycodes(modifiers)?;
        let mut held = Vec::new();
        for keycode in keycodes {
            if let Err(error) = self.send_key(window, keycode, true).await {
                self.release_keys(window, &held).await;
                return Err(error);
            }
            held.push(keycode);
        }
        Ok(held)
    }

    pub(super) async fn release_keys(&self, window: &gtk::Widget, held: &[u8]) {
        for keycode in held.iter().rev() {
            let _ = self.send_key(window, *keycode, false).await;
        }
    }

    pub(super) async fn press(&self, window: &gtk::Widget, chord: &Chord) -> Result<(), String> {
        let injector = self.injector()?;
        let (keycode, needs_shift) = injector.keycode(input::keysym(&chord.key)?)?;
        if needs_shift {
            injector.keycode(input::modifier_keysym(Modifier::Shift)?)?;
        }
        let held = self.hold_keys(window, &chord.modifiers).await?;
        let tapped = self.tap_key(window, keycode, needs_shift).await;
        self.release_keys(window, &held).await;
        tapped
    }

    pub(super) async fn type_character(
        &self,
        window: &gtk::Widget,
        character: char,
    ) -> Result<(), String> {
        let keysym = input::keysym(&character.to_string())?;
        if self.injector()?.keycode(keysym).is_err() {
            return self.press_borrowed(window, keysym).await;
        }
        self.press(
            window,
            &Chord {
                modifiers: Vec::new(),
                key: character.to_string(),
            },
        )
        .await
    }

    /// Types a keysym the layout lacks on a borrowed keycode, which is returned only after
    /// GTK has read the press, so the key is never translated with the restored row.
    async fn press_borrowed(&self, window: &gtk::Widget, keysym: u32) -> Result<(), String> {
        let injector = self.injector()?;
        let borrowed = injector.borrow_keycode(keysym)?;
        let pressed = async {
            self.await_keymap(window, borrowed.keycode, keysym, true)
                .await?;
            self.tap_key(window, borrowed.keycode, false).await
        }
        .await;
        let returned = injector.return_keycode(&borrowed);
        let restored = self
            .await_keymap(window, borrowed.keycode, keysym, false)
            .await;
        pressed.and(returned).and(restored)
    }

    async fn await_keymap(
        &self,
        window: &gtk::Widget,
        keycode: u8,
        keysym: u32,
        lent: bool,
    ) -> Result<(), String> {
        let display = window.display();
        let deadline = Instant::now() + protocol::DEFAULT_WAIT;
        loop {
            let shown = display
                .map_keycode(u32::from(keycode))
                .is_some_and(|keys| keys.iter().any(|(_, key)| key.into_glib() == keysym));
            if shown == lent {
                return Ok(());
            }
            if Instant::now() >= deadline {
                let change = if lent { "the lent" } else { "the returned" };
                return Err(format!(
                    "GTK never read {change} keycode {keycode} for keysym {keysym:#06x}"
                ));
            }
            glib::timeout_future(POLL_INTERVAL).await;
        }
    }

    /// The server times the release `EVENT_GAP` after the press: waiting for a frame could hold
    /// the key past the X server's 660 ms autorepeat delay while the app is busy.
    async fn tap_key(
        &self,
        window: &gtk::Widget,
        keycode: u8,
        needs_shift: bool,
    ) -> Result<(), String> {
        let injector = self.injector()?;
        let shift = if needs_shift {
            self.hold_keys(window, &[Modifier::Shift]).await?
        } else {
            Vec::new()
        };
        let up = Event::Key {
            keycode,
            pressed: false,
        };
        let mut result = injector.send(Event::Key {
            keycode,
            pressed: true,
        });
        if result.is_ok() {
            result = match injector.send_after(up, EVENT_GAP) {
                Ok(()) => {
                    glib::timeout_future(EVENT_GAP).await;
                    injector.sync()
                }
                // A key left down would autorepeat until the next press.
                Err(error) => injector.send(up).and(Err(error)),
            };
            self.pause(window, EVENT_GAP).await;
        }
        self.release_keys(window, &shift).await;
        result
    }
}
