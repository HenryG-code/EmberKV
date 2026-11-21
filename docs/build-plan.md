# Build plan

Everything here is **planned**. A slice becomes fact only when its tests pass and its row in the
progress table is ticked. [Requirements](requirements.md) say what must be true,
[architecture](architecture.md) says how, [research](research.md) says why, and this document says
in what order.

## How to use this plan

Build one slice at a time, each on its own short-lived branch and pull request. Do not start a slice
until the one before it is merged. Each slice lists:

- **Learn:** the Rust and systems concepts the slice introduces. Read about them first.
- **Build:** files, types and functions to create.
- **Tests:** what must be proven, and how.
- **Pitfalls:** mistakes that commonly break this kind of code.
- **Done when:** the exit criteria. Nothing else counts.

Starting prompt for a working session:

```text
Read docs/build-plan.md, docs/requirements.md, docs/architecture.md and docs/research.md.
We are on slice <N>: <name>. Build only that slice.
I develop on Windows 11, editing in VS Code connected to WSL2 Ubuntu, and the repository lives in
the Ubuntu home folder. Give every command for the Ubuntu terminal unless it says PowerShell.
Explain each decision and why, before writing the code for it, including the Rust concepts involved.
Give me one small hands-on task per step to type myself.
Write the tests listed for the slice, and run them.
Update docs/architecture.md, the progress table in docs/build-plan.md and CHANGELOG.md in the same change.
Write an ADR for any decision the slice says to record.
Stop when the slice's "done when" list is met, and tell me what to try by hand.
```

## Development environment on Windows

Step-by-step setup and common problems are in [Development setup](development.md). In short:

EmberKV depends on Linux file-flush behaviour and on killing processes with `kill -9`, so **it is
built and tested inside WSL2 Ubuntu**, not natively on Windows. Windows is where you edit and use the
browser.

| Where | What runs there | Why |
|---|---|---|
| Windows 11 | VS Code (with the WSL extension), browser, GitHub Desktop or Git for Windows if wanted | Your normal desktop |
| WSL2 Ubuntu, `~/projects/emberkv` | Rust toolchain, `cargo`, tests, the server, crash tests | Linux file semantics, `kill -9`, same as CI |
| Laptop, native Ubuntu | Benchmarks and the later power-loss gate | No virtual disk in between, so results mean something |
| GitHub Actions, Ubuntu | CI | Clean, repeatable machine |

**Keep the repository in the Ubuntu home folder, not under `/mnt/c/`.** Files under `/mnt/c/` are on
the Windows disk, reached through a translation layer that is slow and does not behave like a Linux
filesystem when flushing. A durability test there would measure the translation layer, not
EmberKV. The copy in `Desktop\MyAppProjects\GithubProjects\emberkv` becomes a stale reference once
you clone into WSL, the same arrangement as PocketCloud.

The storage crate avoids Linux-only calls where it can, so `cargo test -p emberkv-storage` should also
work on Windows. That is a convenience, not a goal. Crash and durability tests are Linux only and are
marked with `#[cfg(target_os = "linux")]`.

## Decisions to make before slice 1

| Decision | Recommendation | Record in |
|---|---|---|
| Storage design | Log-structured hash table (Bitcask) with an ordered in-memory index | ADR 0001 |
| Record format and checksum | The format in the architecture, with CRC-32C | ADR 0002, after slice 0 |
| Writer model | A dedicated OS thread owns the log; a failed flush is fatal and never retried | ADR 0003 |
| Wire protocol | RESP2 subset plus inline commands, so `redis-cli` and `nc` work in demos | ADR 0004 |
| Dependencies | Tokio, `crc32c`, `tracing`, `thiserror`, `proptest`. Justify each one ([research](research.md), section 6) | ADR 0005 |
| Licence | MIT OR Apache-2.0, chosen | `LICENSE-MIT`, `LICENSE-APACHE` |

Use the latest stable Rust at the time of building (1.100.0 when this was written) and record the
exact version in `rust-toolchain.toml`.

## Slice 0: feasibility prototype

**Goal:** prove the core durability claim before designing anything around it.

**Where:** `prototypes/crash-durability/`, a standalone Cargo project plus shell scripts. Throwaway
code: clarity beats structure, and it is not a workspace member.

**Learn:** `std::fs::File`, `OpenOptions` with append, `write_all`, `sync_all` versus `sync_data`,
why a directory must be flushed after a rename, `u32::to_le_bytes`, `std::process::exit`, signals
and `kill -9`, Bash loops.

**Build, in order:**

1. `writer`: takes a file path and a count. For each number `n`, it builds a record
   `[crc32c: u32][len: u32][payload]` where the payload is `n` as text, appends it, calls `sync_all`,
   and **only then** prints `n` and flushes stdout.
2. `checker`: reads the file from the start, decodes records one at a time, and stops at the first
   one that is short or fails its checksum. It prints the last good number and whether a tail was
   discarded. A bad record followed by good ones prints `CORRUPT` and exits with an error.
3. `kill-loop.sh`: start the writer in the background, sleep a random 10 to 500 milliseconds,
   `kill -9` it, take the last number it printed, and run the checker. Fail if the checker's last
   number is lower. Repeat 500 times and print the totals.
4. `damage.sh`: copy a good file, then (a) cut it with `truncate` part-way through the last record
   and (b) flip one bit in an early record with `printf` and `dd`. Run the checker on each.
5. `rename-test`: write `data.tmp`, flush it, rename it to `data`, flush the directory, with an
   optional crash point after each step (`--crash-after=write|sync|rename|dirsync`). A loop checks
   that `data` is always either the full old content or the full new content.
6. `bench`: write 100,000 records of 100 bytes with a flush on each and without, and print records
   per second. Then insert one million random 16-byte keys into a `BTreeMap<Vec<u8>, (u32, u64)>` and
   report the process's peak memory from `/proc/self/status` (`VmHWM`).
7. A failed-flush simulation: wrap the file in a test helper whose flush returns an error once, and
   show that the writer stops instead of carrying on.

**Pitfalls:**

- Printing before flushing makes the test meaningless. The order is append, flush, then print.
- `kill -9` proves the record format and recovery, not power-loss safety ([research](research.md),
  section 2). Write that in the results.
- Record the filesystem with `df -T .` and whether you ran under `/mnt/c` (you should not have).

**Done when:** all [feasibility gates](feasibility.md) are ticked with a result file in
`docs/benchmarks/` (environment, commands, raw numbers), and ADRs 0001 and 0002 are written.

## Slice 1: workspace skeleton and CI

**Goal:** an empty workspace that builds, lints and tests the same way locally and in CI.

**Learn:** Cargo workspaces, crate types (library and binary), `rust-toolchain.toml`,
`[workspace.lints]`, Clippy, `cargo fmt`, GitHub Actions basics, pinning actions to a commit SHA.

**Build:**

- Root `Cargo.toml` with `members = ["crates/*"]`, `resolver = "3"`, edition 2024, shared
  `[workspace.dependencies]` and `[workspace.lints]` (`unsafe_code = "forbid"`, Clippy `pedantic`
  as warnings).
- `crates/emberkv-storage` (library), `crates/emberkv-server` (binary `emberkv`),
  `crates/emberkv-cli` (binary `emberkv-cli`), each with one placeholder test.
- `rust-toolchain.toml` with the exact version plus `rustfmt` and `clippy` components.
- `deny.toml` that allows only permissive licences and denies known advisories.
- `Makefile`: `test` runs fmt check, clippy with `-D warnings` and `cargo test --workspace`;
  `lint` runs fmt and clippy.
- `.github/workflows/ci.yml`: `permissions: contents: read`, Ubuntu runner, `actions/checkout` pinned
  to a SHA, Rust installed from `rust-toolchain.toml`, cache, `make test`, `cargo deny check`,
  and a Gitleaks secret scan.

**Pitfalls:** do not commit `target/`. Commit `Cargo.lock` because this project ships binaries.

**Done when:** a fresh clone in WSL passes `make test`, CI is green on a pull request, and ADR 0005
lists each dependency with a reason.

## Slice 2: record encoding

**Goal:** turn records into bytes and back, safely, with no possible panic on bad input.

**Learn:** enums with data, `Result` and the `?` operator, `thiserror`, the `Read` trait,
`read_exact` and `UnexpectedEof`, integer overflow and `checked_add`, why to check lengths before
`Vec::with_capacity`.

**Build in `crates/emberkv-storage/src/record.rs`:**

```rust
pub enum Record { Put { key: Vec<u8>, value: Vec<u8> }, Delete { key: Vec<u8> } }

pub enum Decoded {
    Record { record: Record, len: u64 }, // len = bytes consumed
    CleanEnd,                            // zero bytes left
    TornTail { valid_bytes: u64 },       // ran out of bytes mid-record
    Corrupt { offset: u64, reason: CorruptReason },
}

pub struct Limits { pub max_key: u32, pub max_value: u32 }

pub fn encode(record: &Record, out: &mut Vec<u8>) -> Result<(), EncodeError>;
pub fn decode(reader: &mut impl Read, offset: u64, limits: &Limits) -> Result<Decoded, io::Error>;
```

Layout (little-endian): `header_crc: u32 | key_len: u32 | value_len: u32 | body_crc: u32 | key | value`.
A tombstone uses `value_len = u32::MAX` and no value bytes. `header_crc` covers the two lengths and is
checked before the lengths are used; `body_crc` covers key and value. See the architecture for why a
single checksum is not enough.

**Tests (`tests/record.rs` and unit tests):**

- Round trip: proptest with random keys and values up to the limits, `decode(encode(r)) == r`.
- Every truncation point: for one encoded record, cut at each byte length from 1 to its full length
  minus one; every cut returns `TornTail`.
- Every single-bit flip in the header and body returns `Corrupt`, including flips in a length field,
  which must never be reported as `TornTail`.
- Lengths above the limits return `Corrupt` without allocating. Check with a capped test allocator,
  or by asserting that no `Vec` larger than the limit is created.
- Fuzz target `fuzz/fuzz_targets/decode.rs` with `cargo fuzz run decode`, run for 10 minutes.

**Pitfalls:** `key_len + value_len` can overflow `u32`, so add them as `u64`. A length field that
is itself damaged must not cause a huge allocation.

**Done when:** all tests pass, the fuzzer finds nothing in 10 minutes, and ADR 0002 records the final
layout.

## Slice 3: log segments and recovery

**Goal:** an append-only, crash-safe log made of numbered segment files.

**Learn:** `OpenOptions`, `FileExt::read_at` (positioned reads), `set_len` for truncating, directory
flushing, `BufWriter` and why it must be flushed before `sync_all`.

**Build:**

- `segment.rs`: files named `000001.log`, `000002.log` and so on. `SegmentWriter::append(&Record)`
  returns the `(segment_id, offset, len)` where it was written, and `sync()` flushes. When the
  active segment passes `max_segment_bytes` (default 64 MiB), it is flushed and closed, the next
  file is created, and **the directory is flushed** before writes continue.
- `recovery.rs`: `recover(dir, limits) -> Result<RecoveryReport, RecoveryError>` lists segments in
  numeric order and decodes each one. A `TornTail` in the **newest** segment is truncated to
  `valid_bytes` and flushed, and the report records how many bytes were discarded. A `TornTail` or
  `Corrupt` in any older segment, or `Corrupt` anywhere, is an error that stops startup and names
  the file and offset.
- A lock file (`LOCK`, held with an exclusive file lock) so two processes cannot open one data
  directory.

**Tests:**

- The slice 0 kill loop rewritten in `tests/crash/kill_loop.rs` against the real crate (Linux only),
  with the iteration count set by an environment variable: 100 runs in CI, 1,000 locally.
- Rollover: write past three segment sizes, reopen, and check every record.
- Damage tests: torn tail in the newest segment (recovers), torn tail in an older segment (refuses),
  bit flip anywhere (refuses), an empty newest segment (fine), a missing segment number in the
  sequence (refuses).
- Opening the same directory twice fails cleanly.

**Pitfalls:** truncating a torn tail is itself a write, so flush after it. Never "skip" a corrupt record
to carry on: a later record could be an older version of the same key.

**Done when:** acceptance tests 2, 3 and 4 in the requirements pass.

## Slice 4: storage engine

**Goal:** a complete key-value store as a plain synchronous library.

**Learn:** `BTreeMap`, ownership versus borrowing across method calls, `Arc` and `RwLock`, a
dedicated thread with `std::thread::spawn`, channels, model-based testing.

**Build:**

```rust
pub struct Engine { /* index, writer handle, read handles */ }
impl Engine {
    pub fn open(dir: &Path, config: Config) -> Result<(Engine, RecoveryReport), OpenError>;
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, EngineError>;
    pub fn set(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), EngineError>;
    pub fn delete(&self, key: &[u8]) -> Result<bool, EngineError>;
    pub fn exists(&self, key: &[u8]) -> Result<bool, EngineError>;
}
```

- The index maps a key to `{ segment_id, offset, len }` of its newest record. Recovery rebuilds it.
- The **writer thread** owns the active segment. `set` and `delete` send a request with a reply
  channel; the writer appends, calls `sync_all`, updates the index, then replies. The caller returns
  only after the reply, which is what makes an acknowledgement mean "on disk".
- If a flush fails, the writer marks the engine as **failed**: every later write returns
  `EngineError::Failed` and nothing is retried ([research](research.md), section 2).
- `get` reads the index under a read lock, then uses `read_at` on a read handle and verifies the
  checksum before returning the value.

**Tests:**

- Model-based test with `proptest-state-machine`: random sequences of set, delete, get, exists and
  reopen run against the engine and a plain `BTreeMap`, compared after every step.
- An injected flush failure (through a test-only `Storage` trait or a fault flag) makes all later
  writes fail, and reopening recovers every acknowledged write.
- `get` on a record whose bytes were damaged after startup returns an error, never wrong data.

**Pitfalls:** update the index only after the flush succeeds. Hold the index lock for as short a
time as possible, and never while doing disk I/O.

**Done when:** acceptance test 1 passes, the injected-failure test passes, and ADR 0003 is written.

## Slice 5: server and protocol

**Goal:** the engine over the network, speaking RESP2.

**Learn:** async and await, the Tokio runtime, `TcpListener`, spawning tasks, `tokio::sync::mpsc`
and `oneshot`, `spawn_blocking`, back-pressure, `tokio::signal`, `tokio::time::timeout`.

**Build:**

- `emberkv-server/src/resp.rs`: an incremental parser over a `BytesMut` buffer that returns
  `Incomplete`, `Command(Vec<Vec<u8>>)` or `ProtocolError`. It handles arrays of bulk strings and
  inline commands, and checks every declared length against limits before reserving memory.
- Commands: `PING`, `GET`, `SET`, `DEL`, `EXISTS`, `INFO`, plus `HELLO`, which replies with an
  error so clients stay on RESP2. Reply types: `+OK`, `-ERR`, `:n`, `$n` bulk strings and `$-1` null.
- `main.rs`: configuration from command-line flags (`--dir`, `--port 7379`, limits). Listens on
  `127.0.0.1` only unless `--bind` is given. A semaphore caps connections, and idle connections time
  out.
- Connections call the engine through `spawn_blocking` (or a bridge channel to the writer thread),
  never blocking the async runtime.
- Graceful shutdown on Ctrl+C or SIGTERM: stop accepting, let in-flight writes finish, flush, exit.
- `emberkv-cli`: `emberkv-cli set k v`, `get k`, `del k`, and `bench-writes` for the crash harness.
- `make dev-local` runs the server on a temporary data directory; `make stop-dev` sends SIGTERM.

**Tests:**

- Integration tests over real sockets for every command and error reply.
- `redis-cli -p 7379` works by hand (install with `sudo apt install redis-tools`).
- Malformed input: wrong lengths, a missing `\r\n`, huge declared lengths, binary garbage.
- 100 concurrent clients each writing their own keys and reading them back.
- Parser fuzz target, 10 minutes.

**Pitfalls:** a slow client must not hold up others. Never call blocking file I/O directly inside
an async task.

**Done when:** acceptance tests 6 and 7 pass, journeys 1 and 5 work with `emberkv-cli` and
`redis-cli`, and ADR 0004 is written.

## Slice 6: compaction

**Goal:** reclaim disk space without ever risking data.

**Learn:** crash-safe file replacement, generation numbers, measuring garbage.

**Build:**

1. Choose the closed segments to compact (never the active one).
2. Copy each record that is still the live version according to the index into a new segment
   `NNNNNN.log.tmp`. Tombstones are kept only while an older segment might still hold the key.
3. Flush the new file, rename it to its final name, flush the directory.
4. Under the index write lock, point the moved keys at their new positions, unless a newer write
   arrived during compaction (compare positions before replacing).
5. Delete the old segments and flush the directory.
6. On startup, delete any leftover `.tmp` files. If both an old segment and its replacement exist
   after a crash, the replacement's higher number wins, so recovery stays correct.

Trigger with a `COMPACT` command and automatically when garbage passes a threshold (default 50%).

**Tests:**

- A crash injected after each numbered step, then recovery and the model comparison.
- Writes running during compaction are never lost.
- Disk use before and after, recorded in the pull request.

**Pitfalls:** deleting old files before the new one is durable loses data. Dropping a tombstone too
early brings deleted keys back to life.

**Done when:** acceptance test 5 passes, and journey 4 works.

## Slice 7: crash harness and observability

**Goal:** the proof demo, and visibility into what the server is doing.

**Build:**

- `make crash-test`: a script that starts the real server, runs `emberkv-cli bench-writes` (which
  logs each acknowledged key to a file), kills the server with `kill -9` at a random moment,
  restarts it, and checks every acknowledged key. It prints runs, acknowledged writes and lost
  writes.
- `tracing` logs in JSON with startup recovery details, compactions and errors.
- Counters exposed through `INFO`: keys, segments, live and garbage bytes, writes, reads, flush time
  percentiles, recovery time, discarded tail bytes.

**Done when:** the crash test runs at least 1,000 iterations with zero lost writes, and its output
is saved in `docs/benchmarks/`.

## Slice 8: benchmarks and first release

**Build:** `make bench` measuring throughput and p50, p99 and p99.9 latency for reads and flushed
writes, at value sizes of 100 B, 1 KiB and 64 KiB, with 1, 8 and 64 clients. Run it on the laptop's
native Ubuntu. The report states commit, CPU and disk class, Rust version, filesystem, and that each
write was flushed. Then a README quick start, a short recording of the crash demo, an updated
architecture document and changelog, and tag `v0.1.0`.

**Done when:** every acceptance test in the requirements passes in CI, the benchmark report is in
`docs/benchmarks/`, and the pre-publish checklist is complete.

## Later scope, one piece at a time

| Slice | Adds | Evidence required |
|---|---|---|
| 9 | Group commit: one flush for several waiting writes | Higher throughput, crash test still at zero lost writes |
| 10 | Range scans over the ordered index | Property test against the model |
| 11 | Hint files for faster startup | Startup time measured before and after |
| 12 | Power-loss gate with dm-flakey on native Ubuntu | Zero acknowledged writes lost across simulated power cuts |
| 13 | Point-in-time snapshots for backup | Restore test |
| 14 | Single-follower replication experiment | Documented behaviour when the leader crashes |

## Progress

| Slice | Status | Evidence |
|---|---|---|
| 0 Feasibility prototype | Not started | |
| 1 Workspace skeleton and CI | Not started | |
| 2 Record encoding | Not started | |
| 3 Log segments and recovery | Not started | |
| 4 Storage engine | Not started | |
| 5 Server and protocol | Not started | |
| 6 Compaction | Not started | |
| 7 Crash harness and observability | Not started | |
| 8 Benchmarks and first release | Not started | |
