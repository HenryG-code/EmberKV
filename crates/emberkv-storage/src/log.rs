//! The log: a data folder of segments, locked against other processes, recovered on open, and
//! rolled over to a new segment when the active one is full.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::record::{Limits, Record};
use crate::recovery::{RecoveryError, RecoveryReport, recover};
use crate::segment::{Location, SegmentWriter, WriteError, segment_file_name};

/// Settings for a [`Log`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogConfig {
    /// Size limits for keys and values.
    pub limits: Limits,
    /// Once the active segment reaches this size, the next record starts a new segment.
    pub max_segment_bytes: u64,
}

impl Default for LogConfig {
    fn default() -> Self {
        LogConfig {
            limits: Limits::default(),
            max_segment_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Why a log could not be opened.
#[derive(Debug, Error)]
pub enum OpenError {
    /// Another process has this data folder open.
    #[error("{0} is already in use by another EmberKV process")]
    Locked(PathBuf),
    /// Recovery found damage it will not repair on its own.
    #[error(transparent)]
    Recovery(#[from] RecoveryError),
    /// The folder, lock file or active segment could not be opened.
    #[error("disk error opening the log: {0}")]
    Io(#[from] io::Error),
}

/// An open, locked data folder that records can be appended to.
pub struct Log {
    dir: PathBuf,
    config: LogConfig,
    active: SegmentWriter,
    stopped: bool,
    /// Held only so the lock lasts as long as the `Log`. The operating system releases it when the
    /// file is closed, including when the process is killed.
    _lock: File,
}

impl Log {
    /// Opens the data folder `dir`, creating it if needed: takes the lock, runs recovery, calling
    /// `on_record` for every valid record in order, and continues appending to the newest segment.
    ///
    /// # Errors
    ///
    /// [`OpenError::Locked`] if another process holds the folder, [`OpenError::Recovery`] if the
    /// data is damaged in a way a crash cannot explain, [`OpenError::Io`] on a disk error.
    pub fn open(
        dir: &Path,
        config: LogConfig,
        on_record: impl FnMut(Record, Location),
    ) -> Result<(Log, RecoveryReport), OpenError> {
        fs::create_dir_all(dir)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("LOCK"))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(OpenError::Locked(dir.to_path_buf())),
            Err(TryLockError::Error(error)) => return Err(OpenError::Io(error)),
        }

        let report = recover(dir, config.limits, on_record)?;
        let active = match report.segments.last() {
            Some(&id) => {
                let file = OpenOptions::new()
                    .append(true)
                    .open(dir.join(segment_file_name(id)))?;
                SegmentWriter::from_file(file, id, report.newest_size, config.limits)
            }
            None => SegmentWriter::create(dir, 1, config.limits)?,
        };

        let log = Log {
            dir: dir.to_path_buf(),
            config,
            active,
            stopped: false,
            _lock: lock,
        };
        Ok((log, report))
    }

    /// Appends `record`, first starting a new segment if the active one is full.
    ///
    /// Not durable until [`sync`](Self::sync) succeeds.
    ///
    /// # Errors
    ///
    /// As [`SegmentWriter::append`]. A disk error while starting a new segment also stops the log.
    pub fn append(&mut self, record: &Record) -> Result<Location, WriteError> {
        if self.stopped {
            return Err(WriteError::Stopped);
        }
        // A "let chain": both conditions must hold, and the second binds `error` for the body.
        if self.active.size() >= self.config.max_segment_bytes
            && let Err(error) = self.roll_over()
        {
            self.stopped = true;
            return Err(error);
        }
        self.active.append(record)
    }

    /// Flushes every appended record to disk.
    ///
    /// # Errors
    ///
    /// As [`SegmentWriter::sync`].
    pub fn sync(&mut self) -> Result<(), WriteError> {
        if self.stopped {
            return Err(WriteError::Stopped);
        }
        self.active.sync()
    }

    /// The number of the segment records are currently appended to.
    #[must_use]
    pub fn active_segment(&self) -> u32 {
        self.active.id()
    }

    /// Closes the full active segment and starts the next one.
    ///
    /// The old segment is flushed **before** the new one exists. Otherwise a crash could leave the
    /// old segment torn with a newer segment after it, which recovery rightly refuses.
    fn roll_over(&mut self) -> Result<(), WriteError> {
        self.active.sync()?;
        let next = self.active.id() + 1;
        self.active = SegmentWriter::create(&self.dir, next, self.config.limits)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(key: &[u8], value: &[u8]) -> Record {
        Record::Put {
            key: key.to_vec(),
            value: value.to_vec(),
        }
    }

    fn small_segments() -> LogConfig {
        LogConfig {
            max_segment_bytes: 30,
            ..LogConfig::default()
        }
    }

    /// Opens the log and collects the records recovery reported.
    fn open(dir: &Path, config: LogConfig) -> (Log, Vec<(Record, Location)>) {
        let mut seen = Vec::new();
        let (log, _) = Log::open(dir, config, |record, location| {
            seen.push((record, location));
        })
        .unwrap();
        (log, seen)
    }

    #[test]
    fn a_new_folder_gets_a_first_segment_and_a_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let (log, seen) = open(&data, LogConfig::default());

        assert_eq!(log.active_segment(), 1);
        assert_eq!(seen, Vec::new());
        assert!(data.join("000001.log").exists());
        assert!(data.join("LOCK").exists());
    }

    #[test]
    fn records_survive_closing_and_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let (mut log, _) = open(dir.path(), LogConfig::default());
        let first = log.append(&put(b"a", b"1")).unwrap();
        let second = log.append(&put(b"b", b"22")).unwrap();
        log.sync().unwrap();
        drop(log);

        let (mut log, seen) = open(dir.path(), LogConfig::default());
        assert_eq!(
            seen,
            vec![(put(b"a", b"1"), first), (put(b"b", b"22"), second)]
        );
        let third = log.append(&put(b"c", b"3")).unwrap();
        assert_eq!(
            third.offset, 37,
            "appending continues after the recovered records"
        );
    }

    #[test]
    fn a_full_segment_rolls_over_to_the_next() {
        let dir = tempfile::tempdir().unwrap();
        let (mut log, _) = open(dir.path(), small_segments());
        let segments: Vec<u32> = (0..5)
            .map(|i| log.append(&put(&[b'k', i], b"v")).unwrap().segment)
            .collect();
        log.sync().unwrap();

        // 19-byte records, rolling over once a segment reaches 30 bytes: two per segment.
        assert_eq!(segments, vec![1, 1, 2, 2, 3]);
        drop(log);

        let (log, seen) = open(dir.path(), small_segments());
        assert_eq!(seen.len(), 5);
        assert_eq!(log.active_segment(), 3);
    }

    #[test]
    fn a_second_open_of_the_same_folder_is_refused_until_the_first_closes() {
        let dir = tempfile::tempdir().unwrap();
        let (first, _) = open(dir.path(), LogConfig::default());

        let second = Log::open(dir.path(), LogConfig::default(), |_, _| {});
        assert!(matches!(second, Err(OpenError::Locked(_))));

        drop(first);
        assert!(Log::open(dir.path(), LogConfig::default(), |_, _| {}).is_ok());
    }
}
