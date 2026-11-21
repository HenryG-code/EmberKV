# Feasibility gates

**Start here.** Progress is recorded per gate, with results in `docs/benchmarks/`. Work in `prototypes/crash-durability/` with a plain Rust program,
no networking and no index beyond a simple map. Run on Linux, in WSL2 or Ubuntu, and record the
filesystem used.

- [x] Records appended and flushed one at a time survive `kill -9` at random moments, across at least
      hundreds of runs, with every acknowledged record present afterwards: 500 of 500 on ext4 in WSL2,
      process crashes only, see [results](benchmarks/2026-11-21-slice0-wsl2.md)
- [ ] A torn final record, produced by killing mid-write and by truncating the file deliberately, is
      detected by its checksum and discarded, and the records before it are intact
- [ ] A single flipped bit inside an earlier record is detected, and startup refuses to continue
- [ ] A flipped bit in a length field is reported as corruption, not as a torn tail, so recovery never
      truncates valid records after it
- [ ] Writing to a temporary file, flushing, renaming and flushing the directory is shown to leave
      either the old file or the new one after a crash, never a missing or empty file
- [x] Throughput measured with a flush on every write and without, so the cost of durability is known:
      611 against 1.9 million records per second on WSL2, see [results](benchmarks/2026-11-21-slice0-wsl2.md)
- [ ] Memory used by the index measured per million keys, to state the design's key-count limit
- [ ] A simulated flush failure stops all further writes instead of retrying (see [research](research.md), section 2)

A `kill -9` loop proves the record format and recovery, not power-loss durability. Power-loss testing
with dm-flakey on native Ubuntu is a separate, later gate.

Record the results in `docs/benchmarks/` and the record format in ADR 0001 and ADR 0002.
