// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::adapters::local_operations) fn merge_copy(
    progress: Option<Rc<TransferProgressTracker>>,
    names: NamePolicy,
) -> CopyStep {
    Rc::new(move |source, target, copy, cancellable| {
        let (progress, names) = (progress.clone(), names.clone());
        Box::pin(merge_copy_into(
            (source, target),
            copy,
            cancellable,
            (progress, names),
        ))
    })
}

type Copier = (Option<Rc<TransferProgressTracker>>, NamePolicy);

async fn merge_copy_into(
    (source, target): (gio::File, gio::File),
    MergeCopy {
        moving,
        durable,
        placing,
    }: MergeCopy,
    cancellable: gio::Cancellable,
    (progress, names): Copier,
) -> Result<MergeCopied, glib::Error> {
    match (native_path(&source), native_path(&target)) {
        (Some(source_path), Some(target_path)) => {
            let options = copy_engine::Options {
                names,
                manifest: moving,
                durable,
                ..copy_engine::Options::default()
            };
            let copied = copy_engine::copy_local(
                source_path,
                copy_engine::Destination::Merge(target_path, placing),
                options,
                progress,
                &cancellable,
            )
            .await?;
            Ok(MergeCopied {
                sources: copied.manifest,
                nested: copied.nested,
                left: Vec::new(),
            })
        }
        (source_path, _) => {
            let copy = GioMergeCopy {
                source_path,
                moving,
                cancellable,
                progress,
            };
            copy.run((source, target), names).await
        }
    }
}

/// A merge GIO copies, where the source has no local path or the target has none.
struct GioMergeCopy {
    source_path: Option<PathBuf>,
    moving: bool,
    cancellable: gio::Cancellable,
    progress: Option<Rc<TransferProgressTracker>>,
}

impl GioMergeCopy {
    async fn run(
        self,
        (source, target): (gio::File, gio::File),
        names: NamePolicy,
    ) -> Result<MergeCopied, glib::Error> {
        // Recorded before GIO copies it, so a change during the copy shows.
        let sources = match self.source_path.as_ref().filter(|_| self.moving) {
            Some(source_path) => {
                Some(copy_engine::snapshot(source_path.clone(), &self.cancellable).await?)
            }
            None => None,
        };
        let left = LeftOut::default();
        let copy = GioCopy {
            into: CopyInto::Existing,
            cancellable: self.cancellable,
            // GIO provides no creation provenance for destructive undo.
            record: CopyRecord::Nothing,
            progress: self.progress.clone(),
            names: GioNames::new(names, left.clone()),
        };
        copy_tree(source, target, copy).await?;
        let left = match (&sources, self.source_path) {
            (Some(sources), Some(path)) => {
                left.keep_in(sources, &path);
                Vec::new()
            }
            _ if self.moving => left.sources(),
            _ => {
                left.report(self.progress.as_deref());
                Vec::new()
            }
        };
        Ok(MergeCopied {
            sources,
            left,
            ..MergeCopied::default()
        })
    }
}
