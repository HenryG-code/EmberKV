//! The storage engine: a key-value store on top of the [`Log`].
//!
//! - **The index** maps every live key to the [`Location`] of its newest record. It is rebuilt by
//!   recovery on open and lives only in memory.
//! - **Writes** take the writer lock, append, flush, and only then update the index. A write that
//!   returns `Ok` is on disk.
//! - **Reads** look the key up in the index, read that one record from its segment, and check it
//!   before returning the value.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use thiserror::Error;

use crate::log::{Log, LogConfig, OpenError};
use crate::record::{Decoded, Limits, Record, decode};
use crate::recovery::RecoveryReport;
use crate::segment::{Location, WriteError, segment_file_name};

/// Why an engine operation failed.
#[derive(Debug, Error)]
pub enum EngineError {
    /// The write was refused or the log failed. See [`WriteError`].
    #[error(transparent)]
    Write(#[from] WriteError),
    /// A segment could not be read.
    #[error("disk error reading a value: {0}")]
    Io(#[from] io::Error),
    /// The record the index points to is damaged or is not the expected key.
    #[error("the record at segment {:06}, byte {} is damaged", .0.segment, .0.offset)]
    Corrupt(Location),
    /// An earlier operation panicked while holding a lock, so the engine's state cannot be trusted.
    #[error("the engine stopped after an internal error; restart it")]
    Poisoned,
}

/// A key-value store. Every method takes `&self`, so one `Engine` can be shared between threads.
pub struct Engine {
    dir: PathBuf,
    limits: Limits,
    /// Only one writer at a time. Holding this lock also orders index updates the same way as the log.
    writer: Mutex<Log>,
    /// Many readers at a time, or one writer updating it.
    index: RwLock<BTreeMap<Vec<u8>, Location>>,
}

impl Engine {
    /// Opens the data folder, running recovery to rebuild the index.
    ///
    /// # Errors
    ///
    /// As [`Log::open`].
    pub fn open(dir: &Path, config: LogConfig) -> Result<(Engine, RecoveryReport), OpenError> {
        let mut index = BTreeMap::new();
        let (log, report) = Log::open(dir, config, |record, location| match record {
            Record::Put { key, .. } => {
                index.insert(key, location);
            }
            Record::Delete { key } => {
                index.remove(&key);
            }
        })?;
        let engine = Engine {
            dir: dir.to_path_buf(),
            limits: config.limits,
            writer: Mutex::new(log),
            index: RwLock::new(index),
        };
        Ok((engine, report))
    }

    /// The value stored for `key`, or `None` if it has none.
    ///
    /// # Errors
    ///
    /// [`EngineError::Corrupt`] if the stored record fails its checks, [`EngineError::Io`] if it
    /// cannot be read.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, EngineError> {
        let location = self
            .index
            .read()
            .map_err(|_| EngineError::Poisoned)?
            .get(key)
            .copied();
        let Some(location) = location else {
            return Ok(None);
        };
        match self.read_record(location)? {
            Record::Put { key: found, value } if found == key => Ok(Some(value)),
            _ => Err(EngineError::Corrupt(location)),
        }
    }

    /// Whether `key` has a value. Answered from the index alone, without reading the disk.
    ///
    /// # Errors
    ///
    /// [`EngineError::Poisoned`] after an internal error.
    pub fn exists(&self, key: &[u8]) -> Result<bool, EngineError> {
        Ok(self
            .index
            .read()
            .map_err(|_| EngineError::Poisoned)?
            .contains_key(key))
    }

    /// Stores `value` for `key`. Returns only after the write is flushed to disk.
    ///
    /// # Errors
    ///
    /// [`EngineError::Write`] if the record is invalid or the log fails.
    pub fn set(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), EngineError> {
        self.write(Record::Put { key, value })
    }

    /// Removes `key`. Returns whether it existed, only after the deletion is flushed to disk.
    ///
    /// # Errors
    ///
    /// As [`set`](Self::set).
    pub fn delete(&self, key: &[u8]) -> Result<bool, EngineError> {
        // Hold the writer lock while checking, so no other write can slip in between.
        let mut log = self.writer.lock().map_err(|_| EngineError::Poisoned)?;
        if !self.exists(key)? {
            return Ok(false);
        }
        self.write_locked(&mut log, Record::Delete { key: key.to_vec() })?;
        Ok(true)
    }

    fn write(&self, record: Record) -> Result<(), EngineError> {
        let mut log = self.writer.lock().map_err(|_| EngineError::Poisoned)?;
        self.write_locked(&mut log, record)
    }

    /// Appends, flushes, then updates the index, in that order. If the flush fails, the index is
    /// not touched, so no reader can ever see a value that is not on disk.
    fn write_locked(&self, log: &mut Log, record: Record) -> Result<(), EngineError> {
        let location = log.append(&record)?;
        log.sync()?;
        let mut index = self.index.write().map_err(|_| EngineError::Poisoned)?;
        match record {
            Record::Put { key, .. } => {
                index.insert(key, location);
            }
            Record::Delete { key } => {
                index.remove(&key);
            }
        }
        Ok(())
    }

    /// Reads and checks the single record at `location`.
    fn read_record(&self, location: Location) -> Result<Record, EngineError> {
        // A fresh handle per read: each has its own position, so readers never disturb each other.
        let mut file = File::open(self.dir.join(segment_file_name(location.segment)))?;
        file.seek(SeekFrom::Start(location.offset))?;
        let mut reader = file.take(location.len);
        match decode(&mut reader, location.offset, self.limits)? {
            Decoded::Record { record, len } if len == location.len => Ok(record),
            _ => Err(EngineError::Corrupt(location)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::thread;

    use super::*;

    fn open(dir: &Path) -> Engine {
        Engine::open(dir, LogConfig::default()).unwrap().0
    }

    #[test]
    fn set_get_exists_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let engine = open(dir.path());

        assert_eq!(engine.get(b"k").unwrap(), None);
        engine.set(b"k".to_vec(), b"v".to_vec()).unwrap();
        assert_eq!(engine.get(b"k").unwrap(), Some(b"v".to_vec()));
        assert!(engine.exists(b"k").unwrap());

        assert!(engine.delete(b"k").unwrap());
        assert_eq!(engine.get(b"k").unwrap(), None);
        assert!(!engine.exists(b"k").unwrap());
        assert!(
            !engine.delete(b"k").unwrap(),
            "deleting a missing key reports false"
        );
    }

    #[test]
    fn the_newest_value_wins_before_and_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let engine = open(dir.path());
        engine.set(b"k".to_vec(), b"old".to_vec()).unwrap();
        engine.set(b"k".to_vec(), b"new".to_vec()).unwrap();
        engine.set(b"gone".to_vec(), b"x".to_vec()).unwrap();
        engine.delete(b"gone").unwrap();
        assert_eq!(engine.get(b"k").unwrap(), Some(b"new".to_vec()));
        drop(engine);

        let engine = open(dir.path());
        assert_eq!(engine.get(b"k").unwrap(), Some(b"new".to_vec()));
        assert_eq!(
            engine.get(b"gone").unwrap(),
            None,
            "the deletion survives too"
        );
    }

    #[test]
    #[cfg(unix)] // write_all_at is Unix-only; the engine itself is not
    fn a_value_damaged_on_disk_is_an_error_never_wrong_data() {
        use std::os::unix::fs::FileExt;

        let dir = tempfile::tempdir().unwrap();
        let engine = open(dir.path());
        engine.set(b"k".to_vec(), b"value".to_vec()).unwrap();

        // Damage the value's last byte after startup, behind the engine's back.
        let path = dir.path().join("000001.log");
        let size = fs::metadata(&path).unwrap().len();
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        file.write_all_at(b"X", size - 1).unwrap();

        assert!(matches!(engine.get(b"k"), Err(EngineError::Corrupt(_))));
    }

    #[test]
    fn many_threads_can_write_and_read_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let engine = open(dir.path());

        thread::scope(|scope| {
            for t in 0..8u8 {
                let engine = &engine;
                scope.spawn(move || {
                    for i in 0..50u8 {
                        engine.set(vec![t, i], vec![i; 8]).unwrap();
                        assert_eq!(engine.get(&[t, i]).unwrap(), Some(vec![i; 8]));
                    }
                });
            }
        });

        for t in 0..8u8 {
            for i in 0..50u8 {
                assert_eq!(engine.get(&[t, i]).unwrap(), Some(vec![i; 8]));
            }
        }
    }

    #[test]
    fn a_failed_flush_leaves_the_index_alone_and_stops_writes() {
        let dir = tempfile::tempdir().unwrap();
        let engine = open(dir.path());
        engine.set(b"k".to_vec(), b"acknowledged".to_vec()).unwrap();

        engine.writer.lock().unwrap().fail_next_sync = true;
        assert!(engine.set(b"k".to_vec(), b"failed".to_vec()).is_err());
        assert_eq!(
            engine.get(b"k").unwrap(),
            Some(b"acknowledged".to_vec()),
            "a write that failed must not be visible"
        );
        assert!(matches!(
            engine.set(b"other".to_vec(), b"v".to_vec()),
            Err(EngineError::Write(WriteError::Stopped))
        ));
        drop(engine);

        // After a restart, recovery decides. The failed write reached the file before its flush
        // "failed", so it may come back: a failed write is uncertain, never half-applied.
        let engine = open(dir.path());
        let value = engine.get(b"k").unwrap();
        assert!(value == Some(b"acknowledged".to_vec()) || value == Some(b"failed".to_vec()));
        assert_eq!(
            engine.get(b"other").unwrap(),
            None,
            "the refused write never happened"
        );
    }
}
