// SPDX-License-Identifier: MIT

use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

use super::*;

#[test]
fn indexing_waits_for_its_preview_wave_but_not_other_devices_or_later_work() {
    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("context owner");
    let first = begin("gphoto2://phone/DCIM/202606/IMG_0001.JPG");
    let second = begin("gphoto2://phone/DCIM/202605/IMG_0001.JPG");
    let other_device = begin("gphoto2://other/DCIM/IMG_0001.JPG");
    let root = root_uri("gphoto2://phone/");
    let mut pause = Box::pin(wait_for_wave(&root, Duration::from_secs(1)));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(pause.as_mut().poll(&mut cx).is_pending());
    let later = begin("gphoto2://phone/DCIM/later.JPG");
    drop(first);
    assert!(pause.as_mut().poll(&mut cx).is_pending());
    drop(second);
    assert!(matches!(pause.as_mut().poll(&mut cx), Poll::Ready(())));
    drop((later, other_device));
    assert!(ACTIVE.with(|active| active.borrow().is_empty()));
}

#[test]
fn stalled_previews_cannot_hold_indexing_forever_and_cancelled_scans_release_waiters() {
    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("context owner");
    let preview = begin("gphoto2://phone/DCIM/photo.JPG");
    context.block_on(wait_for_wave(
        &root_uri("gphoto2://phone/"),
        Duration::from_millis(1),
    ));
    let root = root_uri("gphoto2://phone/");
    let mut cancelled = Box::pin(wait_for_wave(&root, Duration::from_secs(1)));
    assert!(
        cancelled
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(cancelled);
    drop(preview);
    assert!(ACTIVE.with(|active| active.borrow().is_empty()));
    context.block_on(wait_for_wave(
        &root_uri("gphoto2://phone/"),
        Duration::from_secs(1),
    ));
}

#[test]
fn batch_pause_admits_previews_bound_after_publication() {
    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("context owner");
    let location = Location::uri("gphoto2://phone/");
    let mut pause = Box::pin(yield_after_batch(&location));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(pause.as_mut().poll(&mut cx).is_pending());
    let preview = begin("gphoto2://phone/DCIM/photo.JPG");
    context.block_on(glib::timeout_future(BIND_GRACE + Duration::from_millis(10)));
    assert!(pause.as_mut().poll(&mut cx).is_pending());
    drop(preview);
    assert!(pause.as_mut().poll(&mut cx).is_ready());
}
