# Slice 3 crash test, 21 November 2026

Test `crates/emberkv-storage/tests/crash.rs`. A child process opens the real `Log`, appends records
with values of 0 to 63 bytes, flushes each one and only then prints `ACK n`. It is killed with
`SIGKILL` after 5 to 64 ms. The parent reopens the log, which runs recovery, and checks that the
records read back are exactly 0, 1, 2, ... with none missing or changed, and that every acknowledged
number is among them. Segments are capped at 4 KiB so that crashes also land during rollovers.

## Environment

| Item | Value |
|---|---|
| Machine class | Desktop, Windows 11 with WSL2 Ubuntu |
| Filesystem | ext4 (`target/tmp`, from `CARGO_TARGET_TMPDIR`) |
| Command | `EMBERKV_CRASH_RUNS=500 cargo test -p emberkv-storage --test crash -- crash_loop --nocapture` |

## Result

```text
500 crashes, 5248 records recovered, no acknowledged write lost
test result: ok. 1 passed; 0 failed; finished in 23.06s
```

About 10 records per run: each run lasts 5 to 64 ms, part of which is starting the process and running
recovery, at about 600 flushed writes per second (gate 5). The number is consistent with real flushes.

## A first run that did not count

The first version used `tempfile::tempdir()`, which is under `/tmp`. On this machine `/tmp` is tmpfs,
where a flush does nothing:

```text
500 crashes, 104847 records recovered, no acknowledged write lost
```

That is about 20 times more records than real flushes allow, the same mistake as the slice 0 kill loop.
The test now creates its folder under `CARGO_TARGET_TMPDIR`, on the real disk.

## Does the test catch a real bug?

Mutation check: the child was changed to print `ACK n` **before** appending and flushing record `n`.

```text
run 15: record 278 was acknowledged but only 278 records came back
test result: FAILED
```

So the test fails when an acknowledgement is not backed by a flushed record.

**Scope:** process crashes only, as in slice 0. Power loss needs the later dm-flakey gate on native
Ubuntu.
