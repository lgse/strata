// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::{Rc, Weak}};

use gtk::{gdk, glib, prelude::*};

use crate::{
    app::Browser,
    model::FileEntry,
    services::PreviewProvider,
};

use super::{PreviewDrawer, preview_target};

const SLIDESHOW_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

struct PopupInner {
    window: gtk::Window,
    parent: glib::WeakRef<gtk::ApplicationWindow>,
    drawer: PreviewDrawer,
    play: gtk::ToggleButton,
    browser: RefCell<Option<Weak<Browser>>>,
    slideshow: RefCell<Option<glib::SourceId>>,
}

#[derive(Clone)]
pub struct PreviewPopup {
    inner: Rc<PopupInner>,
}

impl PreviewPopup {
    pub fn new(provider: Rc<dyn PreviewProvider>, parent: &gtk::ApplicationWindow) -> Self {
        let drawer = PreviewDrawer::new(provider, true);
        let window = gtk::Window::builder()
            .transient_for(parent)
            .modal(false)
            .title("Quick Look")
            .default_width(760)
            .default_height(600)
            .build();
        let header = gtk::HeaderBar::builder().title_widget(&gtk::Label::new(Some("Quick Look"))).build();
        let play = gtk::ToggleButton::builder().tooltip_text("Play slideshow").build();
        play.set_child(Some(&crate::assets::chrome_icon(crate::assets::icons::PLAY)));
        let fullscreen = gtk::Button::builder().tooltip_text("Toggle fullscreen").build();
        fullscreen.set_child(Some(&crate::assets::chrome_icon(crate::assets::icons::EXTERNAL_LINK)));
        header.pack_start(&play);
        header.pack_end(&fullscreen);
        window.set_titlebar(Some(&header));
        window.set_child(Some(&drawer.widget()));

        let popup = Self {
            inner: Rc::new(PopupInner {
                window: window.clone(),
                parent: parent.downgrade(),
                drawer: drawer.clone(),
                play: play.clone(),
                browser: RefCell::new(None),
                slideshow: RefCell::new(None),
            }),
        };

        let closed = popup.clone();
        window.connect_close_request(move |_| {
            closed.close();
            glib::Propagation::Stop
        });
        let escaped = popup.clone();
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if escaped.handle_video_key(key, modifiers) {
                return glib::Propagation::Stop;
            }
            if modifiers.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK,
            ) {
                return glib::Propagation::Proceed;
            }
            match key {
                gdk::Key::Escape | gdk::Key::space => {
                    escaped.close();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        window.add_controller(keys);

        let toggled = popup.clone();
        fullscreen.connect_clicked(move |_| toggled.toggle_fullscreen());
        let played = popup.clone();
        play.connect_toggled(move |button| {
            if button.is_active() {
                played.start_slideshow();
            } else {
                played.stop_slideshow();
            }
        });

        popup
    }

    pub fn observe_browser(&self, browser: &Rc<Browser>) {
        self.inner.drawer.observe_browser(browser);
        self.inner.browser.replace(Some(Rc::downgrade(browser)));
        let window = self.inner.window.clone();
        let drawer = self.inner.drawer.clone();
        let weak = Rc::downgrade(browser);
        browser.observe(move |_| {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            let _ = &browser;
            let window = window.clone();
            let drawer = drawer.clone();
            glib::idle_add_local_once(move || {
                window.set_visible(drawer.is_open());
            });
        });
    }

    pub fn is_open(&self) -> bool {
        self.inner.window.is_visible()
    }

    pub fn handle_video_key(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
        self.inner.drawer.handle_video_key(key, modifiers)
    }

    #[cfg(test)]
    pub fn widget(&self) -> gtk::Widget {
        self.inner.drawer.widget()
    }

    #[cfg(test)]
    pub fn window(&self) -> gtk::Window {
        self.inner.window.clone()
    }

    pub fn toggle(&self, entry: Option<FileEntry>, depth: Option<usize>) {
        if self.is_open() {
            self.close();
        } else {
            self.open(entry, depth);
        }
    }

    pub fn open(&self, entry: Option<FileEntry>, depth: Option<usize>) {
        let Some(entry) = entry.and_then(|entry| preview_target(Some(entry))) else {
            return;
        };
        self.inner.drawer.show(entry, depth);
        self.inner.window.present();
    }

    pub fn open_fullscreen(&self, entry: Option<FileEntry>, depth: Option<usize>) {
        self.open(entry, depth);
        if self.is_open() {
            self.inner.window.fullscreen();
        }
    }

    pub fn toggle_fullscreen(&self) {
        if self.inner.window.is_fullscreen() {
            self.inner.window.unfullscreen();
        } else if self.is_open() {
            self.inner.window.fullscreen();
        }
    }

    pub fn close(&self) {
        self.stop_slideshow();
        self.inner.play.set_active(false);
        if self.inner.window.is_fullscreen() {
            self.inner.window.unfullscreen();
        }
        self.inner.drawer.close();
        self.inner.window.set_visible(false);
        if let Some(parent) = self.inner.parent.upgrade() {
            parent.present();
        }
    }

    fn start_slideshow(&self) {
        self.stop_slideshow();
        let Some(browser) = self.inner.browser.borrow().as_ref().and_then(Weak::upgrade) else {
            self.inner.play.set_active(false);
            return;
        };
        crate::assets::set_primary_icon(
            &self.inner.play.child().and_downcast::<gtk::Image>().expect("play icon"),
            crate::assets::icons::PAUSE,
        );
        let weak = Rc::downgrade(&browser);
        let id = glib::timeout_add_local(SLIDESHOW_INTERVAL, move || {
            let Some(browser) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            browser.move_selection(1);
            glib::ControlFlow::Continue
        });
        self.inner.slideshow.replace(Some(id));
    }

    fn stop_slideshow(&self) {
        if let Some(id) = self.inner.slideshow.borrow_mut().take() {
            id.remove();
        }
        if let Some(icon) = self.inner.play.child().and_downcast::<gtk::Image>() {
            crate::assets::set_primary_icon(&icon, crate::assets::icons::PLAY);
        }
    }
}

#[cfg(test)]
mod tests;
