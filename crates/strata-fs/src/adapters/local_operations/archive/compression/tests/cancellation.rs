// SPDX-License-Identifier: MIT

use super::super::{CompressionIo, compress_7z, compress_tar, compress_zip};
use super::*;
use std::io::{Cursor, Seek, SeekFrom};
use std::sync::atomic::AtomicUsize;

struct CancelOnOutput<'a> {
    output: Cursor<Vec<u8>>,
    cancelled: &'a AtomicBool,
}

impl Write for CancelOnOutput<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = self.output.write(bytes)?;
        if self.output.position() > 4096 {
            self.cancelled.store(true, Ordering::Relaxed);
        }
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for CancelOnOutput<'_> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.output.seek(position)
    }
}

#[test]
fn every_encoder_stops_inside_a_member_when_cancelled() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let mut seed = 0x1234_5678_u32;
    let payload: Vec<u8> = (0..3 * 1024 * 1024)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        })
        .collect();
    for (format, password, filename) in [
        (ArchiveFormat::Zip, None, "payload.bin"),
        (ArchiveFormat::Zip, Some("test-password"), "payload.mp4"),
        (ArchiveFormat::SevenZ, None, "payload.bin"),
        (ArchiveFormat::SevenZ, Some("test-password"), "payload.bin"),
        (ArchiveFormat::SevenZ, None, "payload.mp4"),
        (ArchiveFormat::SevenZ, Some("test-password"), "payload.mp4"),
        (ArchiveFormat::Tar, None, "payload.bin"),
        (ArchiveFormat::TarGz, None, "payload.bin"),
        (ArchiveFormat::TarGz, None, "payload.mp4"),
    ] {
        let source = root.path().join(filename);
        fs::write(&source, &payload)?;
        let entries = [source];
        let cancelled = AtomicBool::new(false);
        let progress = Arc::new(AtomicUsize::new(0));
        let output = CancelOnOutput {
            output: Cursor::new(Vec::new()),
            cancelled: &cancelled,
        };
        let result = match format {
            ArchiveFormat::Zip => compress_zip(output, &entries, password, &progress, &cancelled),
            ArchiveFormat::SevenZ => compress_7z(output, &entries, password, &progress, &cancelled),
            ArchiveFormat::Tar => compress_tar(output, &entries, None, &progress, &cancelled),
            ArchiveFormat::TarGz => compress_tar(
                output,
                &entries,
                Some(inspect_archive_sources(&entries, &cancelled)?.gzip_level()),
                &progress,
                &cancelled,
            ),
            ArchiveFormat::Rar => unreachable!("read-only format"),
        };
        assert!(
            cancelled.load(Ordering::Relaxed),
            "encoding must reach the cancellation trigger"
        );
        assert!(
            matches!(result, Err(ArchiveError::Cancelled)),
            "{format:?} {filename}: {result:?}"
        );
        assert_eq!(
            progress.load(Ordering::Relaxed),
            0,
            "must stop before finishing the first member"
        );
    }
    Ok(())
}

#[test]
fn cancelled_compression_io_never_touches_the_source_or_destination() -> Result<(), Box<dyn Error>>
{
    let cancelled = AtomicBool::new(false);
    let mut stream = CompressionIo::new(Cursor::new(b"original".to_vec()), &cancelled);
    let mut byte = [0];
    stream.read_exact(&mut byte)?;
    assert_eq!(byte, [b'o']);
    cancelled.store(true, Ordering::Relaxed);
    assert!(stream.read(&mut byte).is_err());
    assert!(stream.write(b"replacement").is_err());
    assert!(stream.flush().is_err());
    assert!(stream.seek(SeekFrom::Start(0)).is_err());
    assert_eq!(stream.inner.position(), 1);
    assert_eq!(stream.inner.into_inner(), b"original");
    Ok(())
}
