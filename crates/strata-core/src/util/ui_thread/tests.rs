// SPDX-License-Identifier: MIT

use super::{Exception, allow, assert_off, mark};

fn refused() -> bool {
    std::panic::catch_unwind(assert_off).is_err()
}

#[test]
fn the_ui_thread_touches_files_only_inside_an_allowed_exception() {
    let outcomes = std::thread::spawn(|| {
        let before_marking = refused();
        mark();
        let outside_dispatch = refused();
        let allowed = {
            let _allowed = allow(Exception::TestSetup);
            refused()
        };
        (before_marking, outside_dispatch, allowed, refused())
    })
    .join()
    .expect("marked thread");
    assert_eq!(outcomes, (false, true, false, true));
}
