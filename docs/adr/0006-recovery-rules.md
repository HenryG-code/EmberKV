# 0006: Recovery rules

- **Status:** Accepted
- **Date:** 2026-11-21

## Problem

On startup EmberKV reads every segment and rebuilds its index. When a segment does not end cleanly,
recovery must choose between keeping going and refusing to start. Keeping going after real damage loses
or returns bad data silently; refusing after an ordinary crash makes the database unusable for no reason.

## Options

1. **Skip anything that fails a check** and keep every record that passes. Simple, but a skipped record
   could be the newest version of a key, so an older value would quietly come back.
2. **Truncate at the first problem, anywhere.** Always starts, but turns any damage, even in the
   middle of an old file, into silent loss of everything after it.
3. **Accept only the damage a crash can cause, refuse everything else.**

## Decision

Option 3. A crash can only interrupt the last write, so:

- **Segments must be numbered without gaps.** A missing number means a whole file is gone.
- **Only the newest segment may end in a torn tail.** It is cut off with `set_len` and the cut is
  flushed, so the next start finds a clean log.
- **Corruption anywhere stops recovery**, naming the segment, byte offset and reason. Nothing on disk is
  changed.
- **A torn tail is accepted only if no valid record exists after it.** Recovery tries to decode a
  record at every byte position after the torn record's header. Finding one proves the "tear" was a
  damaged length that still matched its checksum (ADR 0002's remaining 1-in-4-billion risk), so
  recovery refuses instead of cutting off good data.

## Consequences

- **Easier:** every ordinary crash recovers automatically, and every other kind of damage is reported
  rather than hidden.
- **A known false alarm:** the scan skips the torn record's own header but not its value. If a value
  happens to contain a complete, valid EmberKV record, for example EmberKV data stored inside EmberKV,
  and the crash tears exactly that value, recovery refuses to start even though the crash was
  ordinary. This was chosen deliberately: a false alarm stops the database and asks a person, while the
  opposite mistake would silently delete acknowledged writes.
- **Cost:** the scan reads the rest of the newest segment once, only when a tear is found. Segments are
  capped at 64 MiB, so it is bounded.
- **Would change the decision:** a need to start unattended after any damage, which would call for
  keeping corrupt segments aside for inspection instead of refusing.
