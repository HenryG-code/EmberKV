# 0002: Record format with separate header and body checksums

- **Status:** Accepted
- **Date:** 2026-11-21

## Problem

After a crash, recovery reads the log from the start and has to tell three situations apart:

- a complete, intact record, which it keeps;
- a **torn tail**, the last record cut short by a crash, which was never acknowledged and can be dropped;
- **corruption**, a damaged record that was acknowledged, where startup must stop rather than guess.

Getting this wrong in one direction returns damaged data. Getting it wrong in the other direction
silently deletes good data.

## Options

1. **No checksum.** A damaged record is returned as if it were valid. Rejected.
2. **One CRC-32C over the whole record** (the first design). The reader has to use the length field to
   know how many bytes to read **before** it can check anything.
3. **A header checksum over the length fields, plus a body checksum over the data.**

## Decision

Option 3. Each record is laid out like this, with all numbers little-endian:

```text
header_crc: u32 | key_len: u32 | value_len: u32 | body_crc: u32 | key | value
```

`header_crc` is checked **before** the lengths are used for anything. A deletion (tombstone) uses
`value_len = u32::MAX` and has no value bytes. The checksum is CRC-32C, which modern CPUs compute in
hardware (see `docs/research.md`, section 3).

Evidence from the feasibility prototype, which used a single length field:

- **Option 2 lost data in a test.** Changing one length byte from 1 to 255 made the reader run past the
  end of the file. It reported "torn tail, recovered", and recovery would have deleted the 9 valid
  records after it.
- **With option 3 the same damage is reported as `CORRUPT: BadHeader`.** A flipped payload bit is
  reported as `BadBody`, both at the start and in the middle of the file, and a truncated final record
  is still recovered as a torn tail.

## Consequences

- **Easier:** a damaged length can no longer pretend to be a crash, and every result names the reason.
- **Costs:** 4 more bytes per record, and a second checksum, which is negligible next to the 1.6 ms
  flush measured in gate 5.
- **Remaining risk:** a damaged length that still matches its 4-byte checksum, about 1 in 4 billion.
  Recovery therefore also refuses a torn tail when any valid record can be found after it, because a
  crash can only interrupt the last write (see `docs/architecture.md`).
- **Would change the decision:** a need to detect deliberate tampering. CRC only detects accidental
  damage; tampering would need a keyed hash.
