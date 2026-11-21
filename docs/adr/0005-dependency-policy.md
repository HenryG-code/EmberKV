# 0005: Dependency policy

- **Status:** Accepted
- **Date:** 2026-11-21

## Problem

Every dependency is code that runs inside a database promising not to lose data, written by someone
else, and updated without our review. Each one can bring bugs, security advisories or licence
problems. The rules for adding dependencies need to exist before the first one is added.

## Options

1. Add crates freely as they become useful.
2. Write everything ourselves.
3. Allow a small, justified set, with automatic checks on every pull request.

## Decision

Option 3.

- **Each dependency is justified in this ADR before it is added**, saying what it does, why the
  standard library is not enough, and which crate it lives in.
- **The storage crate stays as small as possible.** It must not depend on Tokio or any networking, so it
  can be tested and fuzzed as plain code.
- **CI runs `cargo deny check`** on every pull request: known vulnerabilities fail the build, only
  permissive licences are allowed (`deny.toml`), and dependencies must come from crates.io.
- **`Cargo.lock` is committed**, and the build uses `--locked`, so CI builds exactly what was reviewed.
- **The Rust version is pinned** in `rust-toolchain.toml` and changed only in its own pull request.

## Dependencies

Added so far, and planned. `proptest-state-machine` was planned for slice 4 but not needed: a list of
random operations and a loop do the same job with no extra crate.

| Crate | Slice | Used in | Why |
|---|---|---|---|
| `crc32c` | 2 | storage | Hardware CRC-32C; tested in the feasibility prototype |
| `thiserror` | 2 | storage | Typed errors without hand-written boilerplate |
| `proptest` | 2 | tests only | Property tests, and the model-based engine test (slice 4) |
| `tempfile` | 3 | tests only | Temporary folders that are removed after each test |
| `tokio` | 5 | server | Async TCP server: connections, timers, signals, semaphore, channels. Only the features used are enabled |
| `libfuzzer-sys` | 2, 5 | fuzz targets only | Coverage-guided fuzzing; in separate workspaces that never ship |
| `tokio` | 5 | server | TCP server, channels and signals |
| `tracing`, `tracing-subscriber` | 7 | server | Structured logs |

## Consequences

- **Easier:** reviewing what runs, and passing the security and licence checks.
- **Harder:** some small utilities will be written by hand.
- **Would change the decision:** a dependency becoming unmaintained or getting an advisory. It is then
  replaced or removed, recorded in a new ADR.
