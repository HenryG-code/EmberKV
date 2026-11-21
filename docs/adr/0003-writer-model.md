# 0003: One writer at a time, behind a lock; index updated only after the flush

- **Status:** Accepted
- **Date:** 2026-11-21

## Problem

Many clients will read and write at once (slice 5), but the log can only be appended to by one writer:
two writers interleaving bytes in one segment would destroy it. Readers must never see a value that
is not yet on disk, and must not be blocked for long by writers.

## Options

1. **A dedicated writer thread** owning the log, fed by a bounded channel, each request carrying a
   reply channel. This is what the research and the original build plan suggested.
2. **A mutex around the log.** Writers take turns by locking it; readers never touch it.
3. **One lock around everything**, readers included. Simplest, but every read waits for every flush,
   about 1.6 ms each (gate 5).

## Decision

Option 2 for the engine library, with the index behind a separate read-write lock.

- **Writes** lock the `Log`, append, flush, then take the index's write lock briefly to update it.
  The writer lock is held throughout, so index updates happen in the same order as the log.
- **Reads** take only the index's read lock, long enough to copy one `Location`, then read the disk
  without holding any lock. A slow flush never blocks a read.
- **The index changes only after the flush succeeds.** If the flush fails, the write returns an error
  and the index is untouched, so no reader can see a value that might not be on disk.
- **A panic while holding a lock** poisons it, and every later call returns `EngineError::Poisoned`.
  The engine stops rather than continuing with state that may be half-updated.

Why not the writer thread yet: the engine is a synchronous library with no async callers. A mutex
gives the same single-writer guarantee with no extra thread, channel or shutdown logic, and is easier
to test. Option 1 becomes worth its cost with **group commit** (slice 9), where one thread collects
waiting writes and flushes them together. The engine's public methods do not change between the two,
so switching later does not affect callers.

## Consequences

- **Easier:** the engine is `Sync` and can be shared between threads directly; tests use plain threads.
- **Harder:** a writer waiting for the lock blocks its thread. The async server (slice 5) must call
  the engine through `spawn_blocking`, never directly on an async task.
- **A failed write is uncertain:** after a failed flush the record may or may not be on disk. The
  caller is told it failed, and after a restart recovery decides. This is the honest answer to
  fsyncgate; it is documented for clients in slice 5.
- **Would change the decision:** group commit, or measurements showing lock contention between
  writers, which point to option 1.
