//! Segment files: the log is split into numbered files such as `000001.log`, `000002.log`. Records are
//! only ever appended to the newest one, the active segment.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use thiserror::Error;

use crate::record::{EncodeError, Limits, Record, encode};

/// Where a record lives on disk: the value the in-memory index stores for every key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    /// Which segment file.
    pub segment: u32,
    /// Byte position of the record inside it.
    pub offset: u64,
    /// Size of the whole record, header included.
    pub len: u64,
}

/// The file name of segment `id`, for example `000001.log`.
#[must_use]
pub fn segment_file_name(id: u32) -> String {
    format!("{id:06}.log")
}

/// The segment number in a file name such as `000001.log`, or `None` for any other file.
#[must_use]
pub fn parse_segment_file_name(name: &str) -> Option<u32> {
    let digits = name.strip_suffix(".log")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Makes changes to a folder's entries durable, such as a newly created or renamed file.
///
/// A file's contents are flushed with the file, but its name lives in the folder, which needs its own
/// flush.
///
/// # Errors
///
/// Returns an error if the folder cannot be opened or flushed.
#[cfg(unix)]
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// On Windows a folder cannot be opened this way, and NTFS journals folder changes itself.
///
/// # Errors
///
/// Never fails; it returns `Result` to match the Unix version.
#[cfg(not(unix))]
pub fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

/// Something a segment can be written to: a real file, or a fake one in tests.
pub trait LogFile: Write {
    /// Forces everything written so far onto the disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the disk reports a failure.
    fn sync(&mut self) -> io::Result<()>;
}

impl LogFile for File {
    fn sync(&mut self) -> io::Result<()> {
        self.sync_all()
    }
}

/// Why a write to a segment failed.
#[derive(Debug, Error)]
pub enum WriteError {
    /// The record was refused before anything was written. The writer is still usable.
    #[error(transparent)]
    Encode(#[from] EncodeError),
    /// The disk reported an error. The writer has stopped.
    #[error("disk error: {0}")]
    Io(#[from] io::Error),
    /// An earlier write or flush failed, so this writer refuses everything until a restart.
    #[error("an earlier write or flush failed; restart so recovery can check the log")]
    Stopped,
}

/// Appends records to one segment.
///
/// After any disk error the writer stops for good: a failed write may have left part of a record in
/// the file, and a failed flush may have lost data the kernel then reports as written. Only recovery,
/// on the next start, can tell what is really on disk.
pub struct SegmentWriter<F: LogFile = File> {
    file: F,
    id: u32,
    size: u64,
    limits: Limits,
    buf: Vec<u8>,
    stopped: bool,
}

impl SegmentWriter<File> {
    /// Creates segment `id` in `dir` and makes the new file's name durable. Fails if it already exists.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists or cannot be created, or if the folder cannot be flushed.
    pub fn create(dir: &Path, id: u32, limits: Limits) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(dir.join(segment_file_name(id)))?;
        sync_dir(dir)?;
        Ok(SegmentWriter::from_file(file, id, 0, limits))
    }
}

impl<F: LogFile> SegmentWriter<F> {
    /// Wraps an open file that already holds `size` valid bytes of segment `id`.
    #[must_use]
    pub fn from_file(file: F, id: u32, size: u64, limits: Limits) -> Self {
        SegmentWriter {
            file,
            id,
            size,
            limits,
            buf: Vec::new(),
            stopped: false,
        }
    }

    /// Writes `record` to the end of the segment and returns where it went.
    ///
    /// The record is **not durable** until [`sync`](Self::sync) succeeds.
    ///
    /// # Errors
    ///
    /// [`WriteError::Encode`] if the record is invalid, which leaves the writer usable;
    /// [`WriteError::Io`] on a disk error, after which the writer is stopped;
    /// [`WriteError::Stopped`] if an earlier error stopped it.
    pub fn append(&mut self, record: &Record) -> Result<Location, WriteError> {
        if self.stopped {
            return Err(WriteError::Stopped);
        }
        self.buf.clear();
        encode(record, self.limits, &mut self.buf)?;

        if let Err(error) = self.file.write_all(&self.buf) {
            self.stopped = true;
            return Err(WriteError::Io(error));
        }
        let location = Location {
            segment: self.id,
            offset: self.size,
            len: self.buf.len() as u64,
        };
        self.size += location.len;
        Ok(location)
    }

    /// Flushes every appended record to disk. Only after this succeeds may a write be acknowledged.
    ///
    /// # Errors
    ///
    /// [`WriteError::Io`] if the flush fails, after which the writer is stopped and is never flushed
    /// again; [`WriteError::Stopped`] if an earlier error stopped it.
    pub fn sync(&mut self) -> Result<(), WriteError> {
        if self.stopped {
            return Err(WriteError::Stopped);
        }
        if let Err(error) = self.file.sync() {
            self.stopped = true;
            return Err(WriteError::Io(error));
        }
        Ok(())
    }

    /// This segment's number.
    #[must_use]
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Bytes written to this segment so far, which is also where the next record will start.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::record::{Decoded, decode};

    fn put(key: &[u8], value: &[u8]) -> Record {
        Record::Put {
            key: key.to_vec(),
            value: value.to_vec(),
        }
    }

    /// A pretend file: keeps bytes in memory and fails when told to.
    #[derive(Default)]
    struct FakeFile {
        data: Vec<u8>,
        fail_writes: bool,
        fail_sync: bool,
        syncs: u32,
    }

    impl Write for FakeFile {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.fail_writes {
                return Err(io::Error::other("simulated write failure"));
            }
            self.data.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl LogFile for FakeFile {
        fn sync(&mut self) -> io::Result<()> {
            self.syncs += 1;
            if self.fail_sync {
                return Err(io::Error::other("simulated flush failure"));
            }
            Ok(())
        }
    }

    fn fake_writer() -> SegmentWriter<FakeFile> {
        SegmentWriter::from_file(FakeFile::default(), 1, 0, Limits::default())
    }

    #[test]
    fn segment_names_round_trip_and_other_files_are_ignored() {
        assert_eq!(segment_file_name(1), "000001.log");
        assert_eq!(parse_segment_file_name("000001.log"), Some(1));
        assert_eq!(parse_segment_file_name("1234567.log"), Some(1_234_567));
        for other in [
            "",
            ".log",
            "abc.log",
            "+1.log",
            "000001.tmp",
            "LOCK",
            "000001.log.tmp",
        ] {
            assert_eq!(parse_segment_file_name(other), None, "{other:?}");
        }
    }

    #[test]
    fn appends_report_consecutive_locations_and_decode_back() {
        let mut writer = fake_writer();
        let first = writer.append(&put(b"a", b"1")).unwrap();
        let second = writer.append(&put(b"b", b"22")).unwrap();

        assert_eq!(
            first,
            Location {
                segment: 1,
                offset: 0,
                len: 18
            }
        );
        assert_eq!(
            second,
            Location {
                segment: 1,
                offset: 18,
                len: 19
            }
        );
        assert_eq!(writer.size(), 37);

        let mut reader = Cursor::new(&writer.file.data);
        let decoded = decode(&mut reader, 0, Limits::default()).unwrap();
        assert_eq!(
            decoded,
            Decoded::Record {
                record: put(b"a", b"1"),
                len: 18
            }
        );
    }

    #[test]
    fn an_invalid_record_is_refused_but_the_writer_keeps_working() {
        let mut writer = fake_writer();
        assert!(matches!(
            writer.append(&put(b"", b"v")),
            Err(WriteError::Encode(_))
        ));
        assert_eq!(
            writer.append(&put(b"k", b"v")).unwrap().offset,
            0,
            "nothing was written"
        );
    }

    #[test]
    fn a_failed_write_stops_the_writer() {
        let mut writer = fake_writer();
        writer.file.fail_writes = true;
        assert!(matches!(
            writer.append(&put(b"k", b"v")),
            Err(WriteError::Io(_))
        ));

        writer.file.fail_writes = false; // even if the disk recovers...
        assert!(matches!(
            writer.append(&put(b"k", b"v")),
            Err(WriteError::Stopped)
        ));
        assert!(matches!(writer.sync(), Err(WriteError::Stopped)));
    }

    #[test]
    fn a_failed_flush_stops_the_writer_and_is_never_retried() {
        let mut writer = fake_writer();
        writer.append(&put(b"k", b"v")).unwrap();
        writer.file.fail_sync = true;
        assert!(matches!(writer.sync(), Err(WriteError::Io(_))));

        writer.file.fail_sync = false;
        assert!(matches!(writer.sync(), Err(WriteError::Stopped)));
        assert!(matches!(
            writer.append(&put(b"k", b"v")),
            Err(WriteError::Stopped)
        ));
        assert_eq!(writer.file.syncs, 1, "the failed flush must not be retried");
    }

    #[test]
    fn create_makes_a_real_file_and_refuses_to_overwrite_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = SegmentWriter::create(dir.path(), 7, Limits::default()).unwrap();
        writer.append(&put(b"k", b"v")).unwrap();
        writer.sync().unwrap();

        let path = dir.path().join("000007.log");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), writer.size());

        let again = SegmentWriter::create(dir.path(), 7, Limits::default());
        assert_eq!(
            again.err().map(|e| e.kind()),
            Some(io::ErrorKind::AlreadyExists)
        );
    }
}
