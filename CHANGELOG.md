# Changelog

Notable changes are recorded here, following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Project structure and planning documentation.
- Development setup guide for Windows with WSL2, technical research, issue templates and a reviewer
  checklist.
- Feasibility prototype in `prototypes/crash-durability/`: a record writer and checker, kill-loop and
  file-replacement crash tests, index memory measurement, and a writer that stops on a failed flush.
  All eight feasibility gates pass for process crashes on ext4 in WSL2.
- ADR 0001 (log-structured hash table) and ADR 0002 (record format with header and body checksums).
- Cargo workspace with `emberkv-storage`, `emberkv-server` (binary `emberkv`) and `emberkv-cli`, pinned
  Rust 1.100.0, Clippy pedantic lints, `make test` and `make lint`, and CI: format, lint and tests,
  `cargo deny` for licences and advisories, and a Gitleaks secret scan. ADR 0005 (dependency policy).

- `emberkv-storage` record format: `Record` (put and tombstone), `encode` with size limits, and `decode`
  returning a record, a clean end, a torn tail or classified corruption, with every length checked
  before any allocation. Unit tests, property tests and a fuzz target.

- Segment files with a writer that stops on any disk error, recovery that cuts a torn tail only at the
  end of the newest segment and refuses any other damage, and a `Log` with a folder lock and segment
  rollover. A crash test kills a writer process at random and checks every acknowledged write.
  ADR 0006 (recovery rules).

- `Engine` with `get`, `set`, `delete` and `exists`: an in-memory index rebuilt by recovery, writes that
  return only after their flush, the index updated only after a successful flush, and reads that check
  every record. A model-based test compares it with a `BTreeMap` through random operations and
  restarts. ADR 0003 (one writer behind a lock).

- `emberkv` server speaking a subset of RESP2 on 127.0.0.1: `PING`, `GET`, `SET`, `DEL`, `EXISTS`, inline
  commands and pipelining. Limits on connections, request size and idle time; refuses requests from web
  browsers; graceful shutdown on Ctrl+C or SIGTERM. `emberkv-cli` client, `make dev-local` and
  `make stop-dev`, and a parser fuzz target. ADR 0004 (wire protocol).

### Fixed

- A new data folder's name was not flushed, so after a power cut the whole folder could vanish. Its
  parent folder is now flushed when it is created. Segment numbers can no longer wrap around.
- Lowering the configured value or key limit made existing data unreadable: recovery called it
  corrupt and refused to start. Reading now uses a fixed format maximum; configured limits only apply to
  new writes (ADR 0007).
- A damaged length field was mistaken for a torn tail, which would have discarded valid records. Records
  now carry a separate header checksum that is verified before the length is used.
- Review fixes in the prototype: the writer rejects a length that does not fit in 32 bits instead of
  silently truncating it, flushes the folder after creating `data.log` so the new file's name is
  durable, and the index measurement no longer wraps around if memory use drops.
