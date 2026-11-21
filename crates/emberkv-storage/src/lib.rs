//! EmberKV's storage engine: the append-only log, crash recovery and the in-memory index.
//!
//! Plain synchronous code with no networking, so it can be tested and fuzzed on its own.
//! Empty in slice 1; record encoding arrives in slice 2.

/// Version of the on-disk record format this crate reads and writes.
pub const FORMAT_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::FORMAT_VERSION;

    #[test]
    fn format_version_starts_at_one() {
        assert_eq!(FORMAT_VERSION, 1);
    }
}
