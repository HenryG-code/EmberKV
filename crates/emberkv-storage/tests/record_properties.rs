//! Property tests for the record format: proptest generates thousands of random records and checks
//! that rules hold for every one of them, not just for examples we thought of.

// This whole file is test code, where a panic is a test failure. clippy.toml's test exemption only
// covers #[test] functions, not helpers like `encoded`, so allow unwrap here explicitly.
#![allow(clippy::unwrap_used)]

use std::io::Cursor;

use emberkv_storage::record::{Decoded, Limits, Record, decode, encode};
use proptest::prelude::*;
use proptest::sample::Index;
use proptest::test_runner::TestCaseError;

/// Any valid record: keys of 1 to 63 bytes, values of 0 to 4095 bytes, or a deletion.
fn any_record() -> impl Strategy<Value = Record> {
    let key = prop::collection::vec(any::<u8>(), 1..64);
    let value = prop::collection::vec(any::<u8>(), 0..4096);
    prop_oneof![
        (key.clone(), value).prop_map(|(key, value)| Record::Put { key, value }),
        key.prop_map(|key| Record::Delete { key }),
    ]
}

fn encoded(record: &Record) -> Vec<u8> {
    let mut out = Vec::new();
    encode(record, Limits::default(), &mut out).unwrap();
    out
}

fn decode_all(bytes: &[u8]) -> Decoded {
    decode(&mut Cursor::new(bytes), 0, Limits::default()).unwrap()
}

proptest! {
    #[test]
    fn any_record_round_trips(record in any_record()) {
        let bytes = encoded(&record);
        let len = bytes.len() as u64;
        prop_assert_eq!(decode_all(&bytes), Decoded::Record { record, len });
    }

    #[test]
    fn a_log_of_records_reads_back_in_order(records in prop::collection::vec(any_record(), 0..50)) {
        let mut log = Vec::new();
        for record in &records {
            encode(record, Limits::default(), &mut log).unwrap();
        }

        let mut reader = Cursor::new(&log);
        let mut offset = 0;
        for expected in &records {
            match decode(&mut reader, offset, Limits::default()).unwrap() {
                Decoded::Record { record, len } => {
                    prop_assert_eq!(&record, expected);
                    offset += len;
                }
                other => {
                    return Err(TestCaseError::fail(format!("expected a record at {offset}, got {other:?}")));
                }
            }
        }
        prop_assert_eq!(decode(&mut reader, offset, Limits::default()).unwrap(), Decoded::CleanEnd);
        prop_assert_eq!(offset, log.len() as u64);
    }

    #[test]
    fn any_cut_is_a_torn_tail(record in any_record(), cut in any::<Index>()) {
        let bytes = encoded(&record);
        let cut = 1 + cut.index(bytes.len() - 1); // somewhere from 1 to len - 1
        prop_assert_eq!(decode_all(&bytes[..cut]), Decoded::TornTail { valid_bytes: 0 });
    }

    #[test]
    fn any_single_bit_flip_is_corrupt(record in any_record(), bit in any::<Index>()) {
        let mut bytes = encoded(&record);
        let bit = bit.index(bytes.len() * 8);
        bytes[bit / 8] ^= 1 << (bit % 8);
        let result = decode_all(&bytes);
        prop_assert!(matches!(result, Decoded::Corrupt { .. }), "bit {} gave {:?}", bit, result);
    }

    #[test]
    fn random_bytes_never_panic_and_any_record_found_is_canonical(
        bytes in prop::collection::vec(any::<u8>(), 0..256)
    ) {
        if let Decoded::Record { record, len } = decode_all(&bytes) {
            let len = usize::try_from(len).unwrap();
            prop_assert_eq!(&encoded(&record)[..], &bytes[..len]);
        }
    }
}
