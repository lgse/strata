// SPDX-License-Identifier: MIT

mod palette;

use gtk::{
    gdk::{Key, ModifierType},
    glib,
};

use super::*;
use crate::{test_support::gtk_test, ui::browser_modes::BrowserMode};

struct Fixture {
    window: gtk::ApplicationWindow,
    content: WindowContent,
    preferences: Rc<PreferenceManager>,
}

impl Fixture {
    fn new() -> Self {
        let preferences = PreferenceManager::shared();
        let window = gtk::ApplicationWindow::builder()
            .application(&application())
            .default_width(1200)
            .default_height(760)
            .build();
        let content = WindowContent::new(&window, &preferences);
        content.bind(&window, &preferences);
        window.present();
        Self {
            window,
            content,
            preferences,
        }
    }

    fn layer(&self, class: &str) -> Option<gtk::Widget> {
        let mut child = self.content.overlay.first_child();
        while let Some(widget) = child {
            if widget.has_css_class(class) {
                return Some(widget);
            }
            child = widget.next_sibling();
        }
        None
    }

    fn close(self) {
        self.content.connect_cleanup(&self.window);
        self.window.destroy();
    }
}

fn application() -> gtk::Application {
    if let Some(application) = gio::Application::default().and_downcast::<gtk::Application>() {
        return application;
    }
    let application = gtk::Application::new(None::<&str>, gio::ApplicationFlags::NON_UNIQUE);
    application
        .register(None::<&gio::Cancellable>)
        .expect("test application registration");
    application
}
