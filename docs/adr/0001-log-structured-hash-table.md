# 0001: Log-structured hash table (Bitcask design) with an ordered index

- **Status:** Accepted
- **Date:** 2026-11-21

## Problem

EmberKV must never lose a write it has acknowledged, even if the process is killed at any moment, and
its design must be small enough to understand completely. The storage design decides how hard both of
those are, so it has to be chosen before any engine code is written.

## Options

1. **Update in place (a B-tree on disk).** Reads are fast and memory use is small, but a crash in the
   middle of rewriting a page can leave it half old and half new. Making that safe needs a separate
   write-ahead log, page checksums and careful recovery: two hard problems instead of one.
2. **Log-structured merge tree (LSM).** Very fast writes and keys need not fit in memory, but it needs
   sorted tables, several levels, background merging and bloom filters. Too much machinery for a first
   release whose purpose is durability.
3. **Log-structured hash table (Bitcask).** Every write is appended to a log and never changed
   afterwards. An in-memory index maps each key to the position of its newest record. Old records
   are removed later by compaction.

## Decision

Option 3, with an ordered `BTreeMap` index instead of Bitcask's hash table.

Evidence from the feasibility prototype (`docs/benchmarks/2026-11-21-slice0-wsl2.md`):

- **Append-only writes make crashes easy to reason about.** A crash can only damage the end of the
  newest file. 500 out of 500 random `kill -9` runs on ext4 lost no acknowledged write.
- **Replacing whole files is safe with write, flush, rename, flush directory.** 0 out of 200 crashes
  broke a file this way, against 23 out of 30 for rewriting in place. Compaction will rely on this.
- **The index cost is known:** about 95 bytes per 16-byte key, roughly 11 million keys per GB of RAM.
- **The ordered index costs nothing extra at one million keys** (95 bytes per key, against 118 for
  `HashMap` at that size) and keeps range scans possible later.

## Consequences

- **Easier:** recovery is a single forward scan; writes are sequential; reads take one index lookup
  and one disk read.
- **Harder:** every key must fit in memory, and that limit must be documented. Disk use grows until
  compaction runs, which needs its own crash-safety work (slice 6). Startup time grows with the size of
  the log until hint files are added (slice 11).
- **Would change the decision:** a need for more keys than RAM allows, which would point to an LSM
  tree, or a need for multi-key transactions, which this design does not offer.
