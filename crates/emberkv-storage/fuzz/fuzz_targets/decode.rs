//! Fuzz target: feed arbitrary bytes to the decoder, reading records one after another exactly as
//! recovery will. It must never panic, never allocate beyond the limits, and anything it accepts as
//! a record must re-encode to exactly the bytes it was read from.

#![no_main]

use std::io::Cursor;

use emberkv_storage::record::{Decoded, Limits, decode, encode};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_key: 1024,
        max_value: 64 * 1024,
    };
    let mut reader = Cursor::new(data);
    let mut offset: u64 = 0;

    loop {
        match decode(&mut reader, offset, limits) {
            Ok(Decoded::Record { record, len }) => {
                let mut again = Vec::new();
                encode(&record, limits, &mut again).expect("a decoded record must be encodable");
                let start = usize::try_from(offset).unwrap();
                let end = start + usize::try_from(len).unwrap();
                assert_eq!(&again[..], &data[start..end], "decoded record is not canonical");
                offset += len;
            }
            Ok(Decoded::CleanEnd | Decoded::TornTail { .. } | Decoded::Corrupt { .. }) => break,
            Err(error) => panic!("reading from memory cannot fail: {error}"),
        }
    }
});