use std::os::unix::net::UnixStream;

use wayland_backend::client::Backend;

use super::*;

/// A connection with no server behind it, as in the offer tests.
fn nowhere() -> (Backend, Home) {
    let (ours, _theirs) = UnixStream::pair().expect("socket pair");
    let backend = Backend::connect(ours).expect("libwayland-client");
    let home = Home {
        backend: backend.downgrade(),
        ui: Some(std::thread::current().id()),
    };
    (backend, home)
}

/// P0's review, S3: a dead connection forgets the drag over a window and the dropped offers
/// not yet finished, as it forgets selections, so nothing on it is read or finished later.
#[test]
fn a_dead_connection_forgets_drags_and_dropped_offers() {
    let (_backend, home) = nowhere();
    let mut st = State::new(Kind::Shared, home.clone(), Arc::default());
    let mimes = vec!["text/plain".to_owned()];
    st.drag = Some(Drag {
        offer: DragOffer::closed(mimes.clone(), home.clone()),
        window: Some("main".into()),
    });
    st.dropped
        .push(DragOffer::closed(mimes.clone(), home.clone()));
    st.selections.insert(
        (Selection::Clipboard, Route::DataDevice),
        Current {
            offer: WlSelection::new(None, mimes, home),
        },
    );
    st.forget_all();
    assert!(st.drag.is_none(), "the drag over the window is forgotten");
    assert!(st.dropped.is_empty(), "dropped offers are forgotten");
    assert!(st.selections.is_empty());
}

/// Review 1 of P4, should-fix 1: a full socket buffer (`EAGAIN` on the flush) leaves the
/// connection alive, and only libwayland's own error kills it.
#[cfg(feature = "foreign-parent")]
#[test]
fn only_libwaylands_own_error_counts_a_round_trip_failure_as_death() {
    use std::io::{Error as IoError, ErrorKind};

    use wayland_backend::client::WaylandError;
    use wayland_client::DispatchError;

    let io = |k: ErrorKind| DispatchError::Backend(WaylandError::Io(IoError::from(k)));
    assert_eq!(classify(&io(ErrorKind::WouldBlock), None), Failure::Busy);
    assert!(matches!(
        classify(&io(ErrorKind::InvalidData), None),
        Failure::Other(_)
    ));
    assert_eq!(
        classify(&io(ErrorKind::WouldBlock), Some("Broken pipe".into())),
        Failure::Dead("Broken pipe".into()),
        "with libwayland's error set, nothing is transient"
    );
}
