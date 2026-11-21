# EmberKV

A small key-value database server written in Rust. Every write is recorded in an append-only log and
flushed to disk before it is confirmed, so a crash never loses a write the client was told succeeded.

> **Status: feasibility prototype in progress.** The design and plan are written, and slice 0 has
> started in `prototypes/crash-durability/`. No claim here has been tested yet.

## Stack

Rust (stable toolchain, edition 2024), Tokio for networking, a subset of the Redis protocol (RESP2)
so `redis-cli` can talk to it, and Rust's built-in test runner with property-based tests. No external
database, no frontend. Developed and tested on Linux (WSL2 Ubuntu on Windows, or native Ubuntu).

## Build order

1. [Feasibility prototype](docs/feasibility.md): append, flush and recover a log after the process is killed
2. First usable release: `GET`, `SET` and `DELETE` over TCP, durable writes, crash recovery and log compaction
3. Later: range scans, snapshots, a replication experiment

## Quick start

Not available yet. To set up a machine for development, see [Development setup](docs/development.md).
Commands are listed by `make help`.

## Documentation

- [Development setup](docs/development.md): where the code lives, tools, VS Code and WSL, common problems
- [Requirements](docs/requirements.md)
- [Architecture](docs/architecture.md)
- [Threat model](docs/threat-model.md)
- [Feasibility gates](docs/feasibility.md)
- [Build plan](docs/build-plan.md): the order of work, one tested slice at a time
- [Technical research](docs/research.md): why the design choices were made, with sources
- [Decision records](docs/adr/)

## Limitations

A single-node teaching database, not a replacement for PostgreSQL, SQLite or Redis. Durability claims
cover process crashes tested on the documented filesystem; power loss depends on the disk honouring
flush requests, which this project cannot verify on consumer hardware.

## Licence

Copyright 2026 Henry Gagiano. Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT), at your option. See [NOTICE](NOTICE).
