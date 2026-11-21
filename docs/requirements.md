# Requirements

## User and problem

A developer wants a small, understandable database that keeps data safe across crashes, and can
explain exactly why. EmberKV exists to make the durability mechanisms inside real databases visible:
write-ahead logging, flushing, recovery and compaction.

## Essential journeys (first release)

1. Start the server, `SET` a key, `GET` it back, and `DELETE` it
2. Kill the server during a stream of writes, restart it, and find every confirmed write intact
3. Find a corrupted or half-written record at the end of the log, and see the server recover the valid
   records before it and report what it discarded
4. Overwrite and delete many keys, run compaction, and see disk use fall while every live key survives
5. Send an oversized key or value, or a malformed command, and get a clear error without the server
   failing or exhausting memory

## Scope stages

| Stage | Includes |
|---|---|
| Feasibility prototype | Append, flush and recover a log after the process is killed, with checksums detecting a torn tail |
| First usable release | `GET`, `SET`, `DELETE`, `EXISTS` and `PING` over TCP, durable writes, crash recovery, compaction, limits, metrics |
| Later scope | Range scans, point-in-time snapshots, group commit, a replication experiment |

**Excluded:** SQL, transactions across several keys, clustering, sharding, and any claim of surviving
power loss on hardware that ignores flush requests.

## Invariants

- A write is acknowledged only after its record is flushed to disk.
- After any crash, every acknowledged write is readable, and no unacknowledged write is partly visible.
- A record whose checksum fails is never returned to a client.
- Compaction never loses a live key, and a crash during compaction leaves the data readable.
- The server never loads a whole data file into memory, and per-request memory is bounded by the limits.

## Non-functional requirements

- Key and value sizes, connection count and request size are bounded and configurable.
- Listens on localhost by default.
- Set throughput and latency goals after the first benchmark, stating whether each write was flushed.
- Correctness matters more than speed. Every speed optimisation must keep the crash tests passing.

## Acceptance tests

- [ ] A property test of random `SET` and `DELETE` sequences matches an in-memory model after every restart
- [ ] Killing the process at random points during writes, hundreds of times, never loses an acknowledged write
- [ ] A truncated or bit-flipped final record is detected, discarded and reported, and earlier records survive
- [ ] A corrupted record before the end of the log stops startup with a clear error rather than silently skipping data
- [ ] A crash at any step of compaction leaves every live key readable after restart
- [ ] Oversized keys, values and requests are rejected without memory growing beyond the documented bound
- [ ] Several concurrent clients writing different keys all read back their own acknowledged values

Proof demo: a script kills the server mid-write in a loop, restarts it each time, and checks every
acknowledged write, printing the number of runs and the number of lost writes, which must be zero.
