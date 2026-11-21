//! The crash test: a child process appends records to a real `Log`, printing each one only after it
//! is flushed, and is killed at a random moment. Then the log is reopened, and every acknowledged
//! record must come back, in order, with nothing missing in between.
//!
//! The child is this same test program, started again with only `crash_child_writer` selected. Set
//! `EMBERKV_CRASH_RUNS` to change how many crashes to run (20 by default, so CI stays quick).

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::env;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use emberkv_storage::log::{Log, LogConfig};
use emberkv_storage::record::Record;

/// Tells the child process which data folder to write to.
const CHILD_DIR: &str = "EMBERKV_CRASH_CHILD_DIR";

/// Small segments, so crashes also land during rollovers.
fn config() -> LogConfig {
    LogConfig {
        max_segment_bytes: 4096,
        ..LogConfig::default()
    }
}

/// The `n`th record the child writes. Values vary in length so records do too.
fn record(n: u64) -> Record {
    Record::Put {
        key: n.to_le_bytes().to_vec(),
        value: vec![b'x'; usize::try_from(n % 64).unwrap()],
    }
}

/// Reopens the log, checks the records are exactly 0, 1, 2, ... with none missing or changed, and
/// returns how many there are.
fn recovered_count(dir: &Path) -> u64 {
    let mut next = 0;
    let (log, _) = Log::open(dir, config(), |found, _| {
        assert_eq!(found, record(next), "record {next} is missing or wrong");
        next += 1;
    })
    .expect("recovery must succeed after a crash");
    drop(log); // release the lock for the next child
    next
}

/// Runs only as the child: appends forever, printing `ACK n` after record `n` is flushed.
#[test]
#[ignore = "started by crash_loop as a child process"]
fn crash_child_writer() {
    let Ok(dir) = env::var(CHILD_DIR) else {
        return;
    };
    let mut next = 0;
    let (mut log, _) = Log::open(Path::new(&dir), config(), |_, _| {
        next += 1;
    })
    .unwrap();
    loop {
        log.append(&record(next)).unwrap();
        log.sync().unwrap();
        println!("ACK {next}"); // printed only after the flush: this is the acknowledgement
        next += 1;
    }
}

#[test]
fn crash_loop() {
    let runs: u32 = env::var("EMBERKV_CRASH_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20);
    // Not tempfile::tempdir(): that is under /tmp, which on many Linux systems is tmpfs, held in
    // memory, where a flush does nothing (lesson 8). Cargo gives integration tests a folder inside
    // target/, on the real disk.
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let this_program = env::current_exe().unwrap();
    let mut seed = u64::from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos(),
    ) | 1;

    for run in 1..=runs {
        let mut child = Command::new(&this_program)
            .args([
                "crash_child_writer",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_DIR, dir.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        thread::sleep(Duration::from_millis(5 + seed % 60));
        child.kill().unwrap(); // SIGKILL: no cleanup, no final flush
        child.wait().unwrap();

        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        let last_acked = output
            .lines()
            .filter_map(|line| line.strip_prefix("ACK "))
            .filter_map(|number| number.parse::<u64>().ok())
            .max();

        let recovered = recovered_count(dir.path());
        if let Some(last) = last_acked {
            assert!(
                recovered > last,
                "run {run}: record {last} was acknowledged but only {recovered} records came back"
            );
        }
    }

    // If the child never managed to write, every run above passed without testing anything.
    let total = recovered_count(dir.path());
    assert!(
        total > 0,
        "the child never wrote a record, so nothing was tested"
    );
    println!("{runs} crashes, {total} records recovered, no acknowledged write lost");
}
