// SPDX-License-Identifier: MIT

use super::*;
use crate::adapters::{
    directory_summary::summarize_directory,
    gio_close::tests::{Probe, probed_file, probed_folder, pump_until},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Site {
    TransferSize,
    PermanentDelete,
    PermanentDeleteKeeping,
    Summary,
    NewFile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LetGo {
    AfterTheWalk,
    StoppedDuringTheOpen,
}

fn walk(
    site: Site,
    probe: &Arc<Probe>,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = ()>>> {
    let folder = probed_folder(probe);
    match site {
        Site::TransferSize => Box::pin(async move {
            let _ = super::super::transfer_size(folder, cancellable, None).await;
        }),
        Site::PermanentDelete => Box::pin(async move {
            let _ = super::super::deletion::permanently_delete(folder, true, cancellable).await;
        }),
        Site::PermanentDeleteKeeping => Box::pin(async move {
            let kept = Rc::new(vec![folder.child("kept")]);
            let _ =
                super::super::deletion::permanently_delete_keeping(folder, kept, cancellable).await;
        }),
        Site::Summary => Box::pin(async move {
            let _ = summarize_directory(&folder).await;
        }),
        Site::NewFile => {
            let file = probed_file(probe);
            Box::pin(async move {
                let _ = super::super::create_entry::create_empty_file(&file, &cancellable).await;
            })
        }
    }
}

#[test]
fn what_an_operation_lets_go_of_is_closed_on_a_worker() {
    let _serial = serial().expect("main context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("the test owns the main context");
    let ui = thread::current().id();
    let mut closed_on_the_ui_thread = Vec::new();
    for site in [
        Site::TransferSize,
        Site::PermanentDelete,
        Site::PermanentDeleteKeeping,
        Site::Summary,
        Site::NewFile,
    ] {
        for let_go in [LetGo::AfterTheWalk, LetGo::StoppedDuringTheOpen] {
            let probe = Arc::new(Probe::default());
            let cancellable = gio::Cancellable::new();
            if let_go == LetGo::StoppedDuringTheOpen {
                probe.hold();
            }
            let walked = Rc::new(Cell::new(false));
            let task = context.spawn_local({
                let walk = walk(site, &probe, cancellable.clone());
                let walked = walked.clone();
                async move {
                    walk.await;
                    walked.set(true);
                }
            });
            if let_go == LetGo::StoppedDuringTheOpen {
                pump_until(&context, "the open is under way", || probe.answering());
                match site {
                    Site::Summary => {
                        task.abort();
                        walked.set(true);
                    }
                    _ => cancellable.cancel(),
                }
                probe.release();
            }
            drop(task);
            pump_until(&context, "the walk ended and let go", || {
                walked.get() && probe.finalized()
            });
            if probe.closed_on() == Some(ui) {
                closed_on_the_ui_thread.push((site, let_go));
            }
        }
    }
    assert_eq!(closed_on_the_ui_thread, []);
}
