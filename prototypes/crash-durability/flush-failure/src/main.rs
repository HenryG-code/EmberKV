use std::fs::{File, OpenOptions};
use std::io::{self, Write};

// ---------- A trait: "anything that can append bytes and flush them" ----------

// NEW: a trait lists methods a type must have. It says WHAT, not HOW.
trait Storage {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn flush_to_disk(&mut self) -> io::Result<()>;
}

// NEW: teach the standard File type our trait. The real disk.
impl Storage for File {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.write_all(bytes)
    }
    fn flush_to_disk(&mut self) -> io::Result<()> {
        self.sync_all()
    }
}

// ---------- The writer ----------

#[derive(Debug)]
enum WriteError {
    Io(io::Error), // this write failed; the error says why
    Stopped,       // an earlier flush failed, so all writes are refused
}

// NEW: generics. LogWriter works with ANY type S, as long as S implements Storage.
struct LogWriter<S: Storage> {
    storage: S,
    stopped: bool,
}

impl<S: Storage> LogWriter<S> {
    fn new(storage: S) -> Self {               // NEW: Self = "this type", here LogWriter<S>
        LogWriter { storage, stopped: false }
    }

    fn append_record(&mut self, payload: &[u8]) -> Result<(), WriteError> {
        if self.stopped {
            return Err(WriteError::Stopped);   // refuse immediately: no write, no flush
        }
        if let Err(e) = self.write_and_flush(payload) {   // NEW: if let = match on one case
            self.stopped = true;               // THE RULE: one failure stops everything
            return Err(WriteError::Io(e));     // and we never try to flush again
        }
        Ok(())
    }

    fn write_and_flush(&mut self, payload: &[u8]) -> io::Result<()> {
        self.storage.append(payload)?;
        self.storage.flush_to_disk()
    }
}

// ---------- A small demo with the real disk ----------

fn main() -> Result<(), WriteError> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("data.log")
        .map_err(WriteError::Io)?;             // NEW: an enum variant used as a function
    let mut log = LogWriter::new(file);
    for n in 1..=3 {
        log.append_record(format!("{n}\n").as_bytes())?;
        println!("{n} saved");
    }
    Ok(())
}

// ---------- Tests, with a fake disk ----------

#[cfg(test)]                                   // NEW: only compiled when running `cargo test`
mod tests {
    use super::*;                              // NEW: bring in everything from the file above

    // A pretend disk: keeps bytes in memory and fails the flush we choose.
    struct FakeDisk {
        written: Vec<u8>,
        flushes: u32,
        fail_on_flush: u32,                    // 0 = never fail
    }

    impl Storage for FakeDisk {
        fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.written.extend_from_slice(bytes);
            Ok(())
        }
        fn flush_to_disk(&mut self) -> io::Result<()> {
            self.flushes += 1;
            if self.flushes == self.fail_on_flush {
                Err(io::Error::other("simulated disk failure"))
            } else {
                Ok(())
            }
        }
    }

    fn disk_failing_on(flush_number: u32) -> FakeDisk {
        FakeDisk { written: Vec::new(), flushes: 0, fail_on_flush: flush_number }
    }

    #[test]                                    // NEW: marks a test function
    fn writes_succeed_while_the_disk_is_healthy() {
        let mut log = LogWriter::new(disk_failing_on(0));
        for _ in 0..5 {
            assert!(log.append_record(b"x").is_ok());   // NEW: b"x" is a byte string, &[u8]
        }
        assert_eq!(log.storage.flushes, 5);    // one flush per write
    }

    #[test]
    fn a_failed_flush_is_reported_to_the_caller() {
        let mut log = LogWriter::new(disk_failing_on(3));
        assert!(log.append_record(b"1").is_ok());
        assert!(log.append_record(b"2").is_ok());
        assert!(matches!(log.append_record(b"3"), Err(WriteError::Io(_))));   // NEW: matches!
    }

    #[test]
    fn after_a_failed_flush_every_write_is_refused_and_never_retried() {
        let mut log = LogWriter::new(disk_failing_on(1));
        assert!(matches!(log.append_record(b"1"), Err(WriteError::Io(_))));

        for _ in 0..3 {
            assert!(matches!(log.append_record(b"more"), Err(WriteError::Stopped)));
        }
        assert_eq!(log.storage.flushes, 1, "a failed flush must never be retried");
        assert_eq!(log.storage.written, b"1", "refused writes must not reach the disk");
    }
}
