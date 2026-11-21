# 0007: Read with a fixed format maximum, write with configured limits

- **Status:** Accepted
- **Date:** 2026-11-21

## Problem

`Limits` (largest key and value) did two jobs: refusing oversized **new** writes, and bounding memory
when **reading** records from disk during recovery and `get`. Because one setting did both, lowering a
configured limit made existing data unreadable. Found in the slice 4 review and confirmed by
experiment: a 100-byte value written with the default limits, then opened with `max_value: 50`,
produced `segment 000001 is corrupt at byte 0: ValueTooLong` and the database refused to start.

## Options

1. **Keep one setting** and document "never lower the limits". Easy to break by accident, and the
   error message blames the data instead of the setting.
2. **Read with no limit at all.** Old data always opens, but a damaged or hostile length could make
   the reader allocate gigabytes, undoing what the decoder's fuzzing proved.
3. **Two separate limits:** configured limits for new writes, and a fixed format maximum for reading.

## Decision

Option 3. `Limits::FORMAT_MAX` is 64 KiB keys and 64 MiB values, the same as the default segment size.

- Recovery and `get` decode with `FORMAT_MAX`, whatever the configuration.
- New writes are checked against the configured limits.
- `Log::open` refuses configured limits larger than `FORMAT_MAX`, so nothing can be written that could
  not be read back.

## Consequences

- **Easier:** limits can be lowered or raised (up to the maximum) at any time, and old data still opens.
- **Bounded:** reading still never allocates more than about 64 MiB per record, so damaged or hostile
  files cannot exhaust memory.
- **Fixed:** raising `FORMAT_MAX` later is a format change. Lowering it could make old data
  unreadable, so it may only ever be raised, and only in a new ADR.
- **Tests:** `lowering_the_value_limit_keeps_old_data_readable` and
  `limits_beyond_the_format_maximum_are_refused`; reverting the fix makes the first one fail.
