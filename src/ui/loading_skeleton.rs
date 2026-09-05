use gtk::prelude::*;

pub(super) const ROW_COUNT: u32 = 18;

pub(super) fn block(width: i32, height: i32) -> gtk::Box {
    let block = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    block.add_css_class("skeleton-block");
    block.set_size_request(width, height);
    block.set_halign(gtk::Align::Start);
    block.set_valign(gtk::Align::Center);
    block
}

pub(super) fn name_width(index: u32) -> i32 {
    [96, 72, 112, 84, 104, 64][index as usize % 6]
}

pub(super) fn container() -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.add_css_class("loading-skeleton");
    container.set_can_target(false);
    container.set_focusable(false);
    container.update_property(&[gtk::accessible::Property::Label("Loading directory")]);
    container
}

pub(super) fn scroll(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .child(child)
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::External)
        .can_target(false)
        .focusable(false)
        .build()
}

pub(super) fn miller() -> gtk::Box {
    let skeleton = container();
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
    rows.add_css_class("skeleton-miller");
    rows.set_valign(gtk::Align::Start);
    for index in 0..ROW_COUNT {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("file-row");
        row.append(&block(17, 17));
        row.append(&block(name_width(index), 10));
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        row.append(&spacer);
        row.append(&block(24, 8));
        rows.append(&row);
    }
    let scroll = scroll(&rows);
    scroll.set_vexpand(true);
    skeleton.append(&scroll);
    skeleton
}
