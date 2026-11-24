// SPDX-License-Identifier: MIT

use super::*;
use crate::{services::SearchItem, ui::search::SearchDialog};

impl ChooserState {
    pub(super) fn show_global_search(&self) -> bool {
        let browser = self.view.browser();
        if browser.confined_to().is_some() {
            return false;
        }
        let Some(overlay) = self.window.child().and_downcast::<gtk::Overlay>() else {
            return false;
        };
        let Some(root) = overlay.child().and_downcast::<BlurBin>() else {
            return false;
        };
        let dialog = self
            .search
            .get_or_init(|| install(&overlay, &root, &self.view));
        root.set_blurred(true);
        dialog.show_global(Some(browser));
        true
    }

    pub(super) fn hide_global_search(&self, layer: &gtk::Widget) -> bool {
        let search = self.search.get().filter(|search| search.widget() == *layer);
        search.inspect(|search| search.hide()).is_some()
    }
}

fn install(overlay: &gtk::Overlay, root: &BlurBin, view: &BrowserView) -> SearchDialog {
    let on_result = |enter_folders| -> Rc<dyn Fn(SearchItem)> {
        let view = view.downgrade();
        Rc::new(move |item| {
            let Some(view) = view.upgrade() else { return };
            let browser = view.browser();
            if !browser.allows_entry(&crate::ui::browser::search_result_entry(&item)) {
                return;
            }
            let location = Location::local(item.path);
            if enter_folders && item.is_directory {
                browser.navigate(location);
            } else {
                view.reveal_location(location);
            }
        })
    };
    let root = root.downgrade();
    let dialog = SearchDialog::new(
        on_result(true),
        on_result(false),
        Rc::new(move || {
            if let Some(root) = root.upgrade() {
                root.set_blurred(false);
            }
        }),
    );
    overlay.add_overlay(&dialog.widget());
    dialog
}
