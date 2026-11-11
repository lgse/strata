use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

fn counting(n: &Arc<AtomicUsize>) -> Notify {
    let n = n.clone();
    Arc::new(move || {
        n.fetch_add(1, Ordering::SeqCst);
    })
}

#[test]
fn a_dropped_subscription_hears_nothing_more() {
    let subs = Subscribers::default();
    let (kept_n, dropped_n) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let kept = subs.add(counting(&kept_n));
    let dropped = subs.add(counting(&dropped_n));
    subs.emit(vec![Event::DragLeave]);
    assert_eq!(
        (
            kept_n.load(Ordering::SeqCst),
            dropped_n.load(Ordering::SeqCst)
        ),
        (1, 1)
    );
    drop(dropped);
    subs.emit(vec![Event::DragLeave, Event::Drop]);
    assert_eq!(
        (
            kept_n.load(Ordering::SeqCst),
            dropped_n.load(Ordering::SeqCst)
        ),
        (2, 1)
    );
    assert_eq!(kept.drain().len(), 3);
    assert_eq!(
        subs.0.lock().map(|s| s.len()).unwrap_or_default(),
        1,
        "forgotten"
    );
}
