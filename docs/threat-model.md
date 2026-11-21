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
| Panics leaving data half-updated | Writer task design where a panic stops writes, and recovery runs on next start |
| Untrusted dependencies | Few dependencies, `cargo audit` and `cargo deny` in CI |
