# Technical research

Background research done before slice 0, on 21 November 2026. It informs the decisions in the
[build plan](build-plan.md). Nothing here is proven for EmberKV until a feasibility gate runs it.
Versions change, so check each one again when its slice starts.

## 1. Storage design: Bitcask

EmberKV follows the Bitcask design from Basho's Riak database ([paper][bitcask-paper],
[summary][bitcask-arpit], [Riak docs][riak]):

- **Data files** are append-only. Exactly one is active for writes; the rest are closed and immutable.
- **The keydir** is an in-memory table that maps each key to the file, offset and size of its newest record.
  A read is one table lookup plus one disk read.
- **A delete** appends a tombstone record. The next merge removes it.
- **Merge**, which we call compaction, rewrites only the live records from closed files into new files.
- **Hint files** sit beside merged files and hold keys and positions but no values, so startup can
  skip scanning the full data. Riak checks hint files with a CRC as well. This is later scope, slice 11.

**Why this design:** it is the simplest design that is truly crash-safe. Writes only ever append, so
a crash can damage at most the tail of the newest file. The known cost is that every key must fit in
memory. Feasibility gate 6 measures that limit.

**Our deviation:** Bitcask uses a hash table. We plan an ordered `BTreeMap` so that range scans
(slice 10) are possible. Gate 6 measures what that costs in memory.

## 2. Durability on Linux: the hard part

### What a flush really means

- `File::sync_data` (fdatasync) forces a file's data to the disk. `sync_all` (fsync) also forces
  its metadata, such as the file size. For an append-only log the file size changes on every
  write, so **use `sync_all`, or `sync_data` only once space has been reserved ahead of time**.
  Start with `sync_all` and measure the difference.
- A new or renamed file is only safe once **its directory is flushed as well**, because the name
  lives in the directory, not in the file. In Rust: `File::open(dir)?.sync_all()?`. Well-tested
  crates use this order: write the temporary file, flush it, rename it, flush the directory
  ([community practice][rust-atomic]).

### What can go wrong

- **Filesystems differ.** The OSDI 2014 study "All File Systems Are Not Created Equal"
  ([paper][alice]) tested eleven widely used storage systems, including LevelDB, SQLite and
  PostgreSQL. It found 60 crash vulnerabilities, mostly from assumptions about the order in which
  writes reach the disk. **Lesson: only assume what a flush guarantees, and test by crashing.**
- **fsyncgate.** On Linux, when a flush fails, the kernel can mark the failed pages as clean. A
  retried flush then reports success without the data ever being written. PostgreSQL lost data this
  way and now stops the server on any flush failure ([study][fsyncgate]).
  **Decision for EmberKV: a failed flush is fatal.** Stop accepting writes, report the error, and
  rely on recovery at the next start. Never retry the flush. This becomes an invariant and a test.

### Process crash versus power loss

These are two different claims, and the documentation must not mix them up:

| Failure | What survives | How to test | Where |
|---|---|---|---|
| Process killed (`kill -9`) | Everything the kernel accepted, flushed or not | Kill loop, slice 0 | WSL2 or Ubuntu |
| Power loss or kernel crash | Only flushed data | dm-flakey or dm-log-writes ([fstests discussion][dmflakey], [log-writes][logwrites]) | Native Ubuntu, later |

A `kill -9` test alone cannot prove that flushing works, because the kernel keeps unflushed writes
in memory after the process dies. It proves the record format and the recovery logic. Proving the
flushes needs a power-loss simulation. dm-flakey drops all writes after a chosen moment;
dm-log-writes records every write and replays to each flush point.

**WSL2:** each distribution's files live on ext4 inside a virtual disk file (VHDX)
([Microsoft docs][wsl-disk]). Microsoft does not document whether a flush inside WSL2 reaches the
physical disk, and there are reports of the virtual disk being corrupted after crashes
([issue][wsl-issue]). **Use WSL2 for development and the kill loop. Make power-loss claims only from
native Ubuntu on the laptop**, and record the filesystem in every benchmark.

## 3. Checksums: CRC-32C, not CRC-32

The two variants use different polynomials. CRC-32 (IEEE) is the zip and gzip checksum.
**CRC-32C (Castagnoli)** is the one used by ext4, btrfs and iSCSI, and modern CPUs compute it in
hardware ([comparison][crc-bench]).

- Candidate crates: [`crc32c`][crc32c], which uses SSE 4.2 on x86-64, or [`crc-fast`][crcfast],
  which covers both variants.
- **Decision: CRC-32C**, recorded in ADR 0002. The architecture said CRC32; this refines it.
- A CRC detects accidental damage, not deliberate tampering. The threat model says so.

## 4. Wire protocol: RESP2 subset

RESP is the Redis protocol ([official spec][resp], [Valkey spec][valkey]):

- Clients send commands as an **array of bulk strings**, for example `*2\r\n$3\r\nGET\r\n$1\r\nk\r\n`.
- Replies use simple strings (`+OK`), errors (`-ERR ...`), integers (`:1`), bulk strings and null.
- **Inline commands**: if the first byte is not `*`, the line is plain space-separated text. This
  lets `telnet` or `nc` work for demos.
- RESP3 adds maps, booleans and other types and is negotiated with `HELLO`. **EmberKV speaks only
  RESP2** and answers `HELLO 3` with an error, so `redis-cli` falls back to RESP2.

Parsing rule from the threat model: check each declared length against a limit **before**
allocating any memory for it.

## 5. Concurrency with Tokio

- **Tokio** latest is 1.53.x. **1.51.x is the long-term support line, until March 2027**
  ([docs.rs][tokio], [release overview][tokio-lts]). Recommendation: use the latest 1.x, keep
  `Cargo.lock` committed, and let `cargo deny` flag advisories.
- **Important:** Tokio's file operations run on a pool of background threads (`spawn_blocking`),
  because Linux file I/O does not really run asynchronously. So **the writer that owns the log
  should be a dedicated operating-system thread**, not an async task. Connections stay async and
  send writes to that thread through a bounded channel, and each one waits for a reply on a oneshot
  channel. That gives the same design as the architecture, with fewer hidden thread hops. Record
  this in ADR 0003.
- **Reads** can run in parallel. Open separate read handles and use positioned reads
  (`FileExt::read_at`), so readers never share a file cursor with the writer.

## 6. Toolchain and dependencies

| Item | Choice | Why |
|---|---|---|
| Rust | Latest stable is 1.100.0 (12 November 2026) ([release notes][rust]); pin it in `rust-toolchain.toml` | Reproducible builds |
| Edition | 2024 | Current edition |
| Dependency cooldown | Cargo 1.100 can skip versions newer than a minimum age when resolving ([release notes][rust]) | Protects against freshly published malicious versions |
| `tokio` | Network server and channels | Standard async runtime |
| `crc32c` | Record checksums | Hardware CRC-32C |
| `tracing`, `tracing-subscriber` | Structured logs | Standard |
| `thiserror` | Error types in the storage crate | Keeps errors typed |
| `proptest`, `proptest-state-machine` | Model-based tests ([state machine guide][proptest-sm]) | Engine against a `BTreeMap` model |
| `cargo-fuzz` | Decoder and parser fuzzing | Finds panics on hostile input |
| `cargo-deny`, `cargo-audit` | Licences, advisories, duplicates | Supply chain |
| `criterion` or a custom harness | Benchmarks | Percentiles, not averages |

Keep the storage crate free of Tokio, so it can be tested and fuzzed as plain synchronous code.

## 7. Changes this research makes to the plan

1. Checksum is **CRC-32C** (ADR 0002).
2. **A failed flush is fatal and never retried.** This adds invariant 6 and an acceptance test that
   injects a flush error.
3. The writer is a **dedicated thread**, not a Tokio task (ADR 0003).
4. **Use `sync_all` on the log** until a measurement justifies `sync_data`.
5. The kill loop proves recovery. **Power-loss durability is a separate, later gate on native Ubuntu**
   using dm-flakey or dm-log-writes.
6. **RESP2 only, with inline commands**, and an error reply to `HELLO 3`.

## Sources

[bitcask-paper]: https://sources.debian.org/src/erlang-bitcask/2.0.8%2Bdfsg-2/doc/bitcask-intro.pdf
[bitcask-arpit]: https://arpitbhayani.me/blogs/bitcask
[riak]: https://docs.riak.com/riak/kv/2.0.4/setup/planning/backend/bitcask/
[rust-atomic]: https://users.rust-lang.org/t/correct-way-to-save-a-file-atomically-but-without-interferring-with-performance/89223
[alice]: https://research.cs.wisc.edu/adsl/Publications/alice-osdi14.html
[fsyncgate]: https://par.nsf.gov//servlets/purl/10176538
[dmflakey]: https://listman.redhat.com/archives/dm-devel/2018-March/033923.html
[logwrites]: https://listman.redhat.com/archives/dm-devel/2015-March/023250.html
[wsl-disk]: https://learn.microsoft.com/windows/wsl/disk-space
[wsl-issue]: https://github.com/microsoft/WSL/issues/5895
[crc-bench]: https://andresilva.github.io/crc32-bench/report
[crc32c]: https://docs.rs/crate/crc32c/^0.6.5
[crcfast]: https://github.com/awesomized/crc-fast-rust
[resp]: https://redis.io/docs/latest/develop/reference/protocol-spec/
[valkey]: https://valkey.io/topics/protocol/
[tokio]: https://docs.rs/crate/tokio/latest
[tokio-lts]: https://socket.dev/cargo/package/tokio
[rust]: https://blog.rust-lang.org/2026/11/12/Rust-1.100.0/
[proptest-sm]: https://proptest-rs.github.io/proptest/proptest/state-machine.html

- [Bitcask paper][bitcask-paper], [Bitcask summary][bitcask-arpit], [Riak Bitcask docs][riak]
- [Atomic file writes in Rust, forum thread][rust-atomic]
- [All File Systems Are Not Created Equal, OSDI 2014][alice]
- [fsync failure study][fsyncgate]
- [dm-flakey power-loss test][dmflakey], [dm-log-writes patches][logwrites]
- [WSL disk space and VHDX][wsl-disk], [WSL corruption issue][wsl-issue]
- [CRC32 benchmark][crc-bench], [crc32c crate][crc32c], [crc-fast][crcfast]
- [RESP specification][resp], [Valkey protocol][valkey]
- [Tokio on docs.rs][tokio], [Tokio releases][tokio-lts]
- [Rust 1.100.0 release][rust]
- [Proptest state machine testing][proptest-sm]
