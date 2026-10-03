// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*, subclass::prelude::*};

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::MenuOption)]
    pub struct MenuOption {
        #[property(get, set)]
        menu_description: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MenuOption {
        const NAME: &'static str = "StrataMenuOption";
        type Type = super::MenuOption;
        type ParentType = gtk::Button;
    }

    #[glib::derived_properties]
    impl ObjectImpl for MenuOption {}
    impl WidgetImpl for MenuOption {}
    impl ButtonImpl for MenuOption {}
}

glib::wrapper! {
    pub struct MenuOption(ObjectSubclass<imp::MenuOption>)
        @extends gtk::Button, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Actionable;
}

pub(super) fn menu_item_button() -> gtk::Button {
    glib::Object::builder::<MenuOption>()
        .property("accessible-role", gtk::AccessibleRole::MenuItem)
        .property("has-frame", false)
        .build()
        .upcast()
}
