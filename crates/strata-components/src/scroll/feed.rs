//! [`ScrollFeed`]: a viewport's latest scroll position, shared by the scroll bar and autoscroll.

use std::cell::Cell;
use std::rc::Rc;

use rune::ScrollPos;

/// The latest [`ScrollPos`] a viewport reported, kept where a handler without the app's state
/// can read it. A host fills it from the viewport's `on_scroll_offset` (or the collection's
/// `Collection::on_scroll`), and autoscroll reads where the viewport stands and how far it can
/// go from it. Clones share one value.
///
/// Used by: C24, C2, C3, C4, hosts.
#[derive(Clone, Default)]
pub struct ScrollFeed(Rc<Cell<ScrollPos>>);

impl ScrollFeed {
    /// A feed that has heard nothing yet: offset, extent and size all zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// The position last heard.
    pub fn get(&self) -> ScrollPos {
        self.0.get()
    }

    /// Records `p`.
    pub fn set(&self, p: ScrollPos) {
        self.0.set(p);
    }

    /// A handler for `El::on_scroll_offset` that records each position.
    pub fn watch(&self) -> impl Fn(ScrollPos) + 'static {
        let feed = self.clone();
        move |p| feed.set(p)
    }
}

impl std::fmt::Debug for ScrollFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let p = self.get();
        f.debug_struct("ScrollFeed")
            .field("offset", &p.offset)
            .field("max", &p.max)
            .finish()
    }
}
