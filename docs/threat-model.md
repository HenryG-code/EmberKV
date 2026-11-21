# Threat model

Initial draft. Update it as the implementation takes shape.

**Valuable data:** stored keys and values, and the integrity of the data directory.

**Entry points:** the TCP listener, the data directory on disk, configuration.

| Threat | Planned mitigation |
|---|---|
| Unauthenticated network access | Listen on localhost only by default. Any wider binding requires an explicit setting and a documented password option |
| Memory exhaustion through huge keys, values or requests | Limits checked before allocating, while reading the request |
| Connection floods | A maximum connection count and idle timeouts |
| Disk filling | A free-space floor that refuses writes before the disk is full, reported as an error |
| Malformed protocol input crashing the server | Parser fuzz tests; errors end the request, not the process |
| Tampered or corrupted data files | Checksums on every record; damage outside the log tail stops startup |
| A crafted segment that makes startup slow | When the newest segment looks torn, recovery tries to decode at every byte after it (ADR 0006). A file with many forged header checksums could make that slow. Bounded by the 64 MiB segment size; accepted, since anyone who can write the data folder can also delete it |
| Panics leaving data half-updated | Writer task design where a panic stops writes, and recovery runs on next start |
| Untrusted dependencies | Few dependencies, `cargo audit` and `cargo deny` in CI |
