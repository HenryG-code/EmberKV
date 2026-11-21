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

### Fixed

- A damaged length field was mistaken for a torn tail, which would have discarded valid records. Records
  now carry a separate header checksum that is verified before the length is used.
- Review fixes in the prototype: the writer rejects a length that does not fit in 32 bits instead of
  silently truncating it, flushes the folder after creating `data.log` so the new file's name is
  durable, and the index measurement no longer wraps around if memory use drops.
