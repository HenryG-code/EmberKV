Closes #

## What changed and why

## Tradeoffs and alternatives

## How it was validated

## Migration or deployment implications

## Self-review

- [ ] Tests cover the changed behaviour, including failure paths
- [ ] Architecture docs and decision records match this change
- [ ] Changelog updated if users would notice
- [ ] No secrets, personal data or real infrastructure details
- [ ] I can explain every line, including generated code

## Review checklist (reviewer)

- [ ] Correctness: every acknowledged write is flushed first; no path skips a checksum
- [ ] Untrusted input: every length or size read from disk or network is checked before use
- [ ] Errors: no `unwrap` or `expect` on data from disk or network; flush errors are fatal, never retried
- [ ] Tests: failure paths covered, and the test would fail if the change were wrong
- [ ] Evidence: numbers state the filesystem and machine; gates link their results
- [ ] Readability: names, comments and structure explain why, not only what
