// SPDX-License-Identifier: MIT

use super::*;

impl PreviewState {
    pub(super) fn render_archive_password_prompt(
        self: &Rc<Self>,
        entry: FileEntry,
        error: Option<&str>,
    ) {
        let focus_prompt = self
            .focus_archive_request
            .take()
            .is_some_and(|request| self.current_request.get() == Some(request));
        self.clear_content();
        self.content_type.set_text(file_extension(&entry));
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
        box_.add_css_class("preview-archive-password");
        box_.set_halign(gtk::Align::Center);
        box_.set_valign(gtk::Align::Center);
        box_.set_vexpand(true);

        let icon = crate::assets::primary_icon(crate::assets::icons::LOCK, 34);
        icon.add_css_class("preview-feedback-icon");
        box_.append(&icon);
        let heading = gtk::Label::new(Some(&crate::i18n::tr("Password-protected archive")));
        heading.add_css_class("preview-feedback-title");
        heading.set_wrap(true);
        heading.set_justify(gtk::Justification::Center);
        box_.append(&heading);
        let detail = gtk::Label::new(Some(&crate::i18n::tr(
            "Enter the password to preview the archive contents",
        )));
        detail.add_css_class("preview-feedback-detail");
        detail.set_wrap(true);
        detail.set_justify(gtk::Justification::Center);
        box_.append(&detail);

        let password = form_password_entry();
        password.set_show_peek_icon(true);
        password.set_placeholder_text(Some(&crate::i18n::tr("Password")));
        // Narrow panes shrink the entry rather than overflowing the window.
        password.set_width_chars(8);
        password.set_max_width_chars(24);
        let unlock = gtk::Button::with_label(&crate::i18n::tr("Unlock"));
        unlock.add_css_class("suggested-action");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.add_css_class("preview-archive-password-row");
        row.set_halign(gtk::Align::Center);
        row.append(&password);
        row.append(&unlock);
        box_.append(&row);

        let weak = Rc::downgrade(self);
        let unlock_entry = entry.clone();
        unlock.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                let password = SecretString::new(
                    state
                        .password_entry
                        .borrow()
                        .as_ref()
                        .map(|entry| entry.text().to_string())
                        .unwrap_or_default(),
                );
                state.focus_archive_on_ready.set(true);
                state.load_with_password(unlock_entry.clone(), 0, Some(password));
            }
        });
        let weak = Rc::downgrade(self);
        let activate_entry = entry.clone();
        password.connect_activate(move |entry| {
            if let Some(state) = weak.upgrade() {
                let password = SecretString::new(entry.text().to_string());
                state.focus_archive_on_ready.set(true);
                state.load_with_password(activate_entry.clone(), 0, Some(password));
            }
        });

        if let Some(error) = error {
            let error_label = gtk::Label::new(Some(error));
            error_label.add_css_class("preview-archive-password-error");
            error_label.set_wrap(true);
            box_.append(&error_label);
        }

        self.password_entry.replace(Some(password.clone()));
        self.content.append(&box_);
        if focus_prompt {
            password.grab_focus();
        } else {
            self.hand_keys_to(&password);
        }
        self.reassert_keyboard_owner();
    }

    pub(super) fn render_archive(self: &Rc<Self>, tree: ArchivePreviewTree, focus_tree: bool) {
        let weak = Rc::downgrade(self);
        let navigate = Rc::new(move |depth: usize| {
            if let Some(state) = weak.upgrade() {
                state.navigate_archive(depth);
            }
        });
        let weak = Rc::downgrade(self);
        let open = Rc::new(move |index: usize| {
            if let Some(state) = weak.upgrade() {
                state.open_archive_row(index as u32);
            }
        });
        let browser = archive::ArchiveBrowser::new(tree, navigate, open);
        if !focus_tree && !self.content.has_focus() {
            browser.clear_selection();
        }
        let list = browser.list().clone();
        self.content.append(browser.root());
        let weak = Rc::downgrade(self);
        // GTK activates on a double-click's second press; the first release already opened it.
        list.connect_activate(move |_, position| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let pointer_pending = state
                .archive_browser
                .borrow()
                .as_ref()
                .is_some_and(archive::ArchiveBrowser::pointer_activation_pending);
            if !pointer_pending {
                state.open_archive_row(position);
            }
        });
        self.archive_browser.replace(Some(browser));
        if focus_tree {
            list.grab_focus();
        } else {
            self.hand_keys_to(&list);
        }
    }

    pub(super) fn navigate_archive(self: &Rc<Self>, depth: usize) {
        if let Some(browser) = self.archive_browser.borrow_mut().as_mut() {
            browser.navigate_to(depth);
        }
        self.reassert_keyboard_owner();
    }

    pub(super) fn archive_key(&self, motion: ArchiveMove) -> bool {
        let mut browsers = self.archive_browser.borrow_mut();
        let Some(browser) = browsers.as_mut() else {
            return false;
        };
        match motion {
            ArchiveMove::Previous => {
                browser.move_cursor(-1);
            }
            ArchiveMove::Next => {
                browser.move_cursor(1);
            }
            ArchiveMove::Parent => {
                if !browser.go_up() {
                    return false;
                }
            }
            ArchiveMove::Open => {
                browser.open_cursor();
            }
        }
        drop(browsers);
        self.reassert_keyboard_owner();
        true
    }

    pub(super) fn archive_list_has_focus(&self, focused: Option<&gtk::Widget>) -> bool {
        self.archive_browser
            .borrow()
            .as_ref()
            .is_some_and(|browser| {
                focused.is_some_and(|focused| {
                    focused == browser.list().upcast_ref::<gtk::Widget>()
                        || focused.is_ancestor(browser.list())
                })
            })
    }

    pub(super) fn close_archive(self: &Rc<Self>) -> bool {
        if self.archive_browser.borrow().is_some() {
            self.close();
            true
        } else {
            false
        }
    }

    pub(super) fn open_archive_row(self: &Rc<Self>, position: u32) {
        let mut browsers = self.archive_browser.borrow_mut();
        let Some(browser) = browsers.as_mut() else {
            return;
        };
        browser.open_child(position as usize);
        drop(browsers);
        self.reassert_keyboard_owner();
    }
}
