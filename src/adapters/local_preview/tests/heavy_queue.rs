// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::PrivateContext;

fn assert_heavy_queue_is_idle() {
    HEAVY_PREVIEW_QUEUE.with(|queue| {
        let queue = queue.borrow();
        assert_eq!(queue.running, 0);
        assert!(queue.queued.is_empty());
    });
}

#[test]
fn pdf_renders_wait_for_the_active_renderer_and_resume_in_order() {
    let context = PrivateContext::new();
    context.block_on(async {
        let first = request_heavy_preview_permit()
            .acquire()
            .await
            .expect("first PDF render permit");
        let mut second = request_heavy_preview_permit();
        let mut third = request_heavy_preview_permit();

        assert!(heavy_preview_is_waiting(&mut second));
        assert!(heavy_preview_is_waiting(&mut third));

        drop(first);
        let second = second.acquire().await.expect("second PDF render permit");
        assert!(heavy_preview_is_waiting(&mut third));
        drop(second);
        drop(third.acquire().await.expect("third PDF render permit"));
    });

    assert_heavy_queue_is_idle();
}

#[test]
fn dropping_a_queued_pdf_render_removes_it_without_consuming_a_slot() {
    let context = PrivateContext::new();
    context.block_on(async {
        let first = request_heavy_preview_permit()
            .acquire()
            .await
            .expect("first PDF render permit");
        let cancelled = request_heavy_preview_permit();
        drop(cancelled);
        drop(first);
    });

    assert_heavy_queue_is_idle();
}
