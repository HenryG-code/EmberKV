//! The on-disk record format, as decided in ADR 0002.
//!
//! ```text
//! header_crc: u32 | key_len: u32 | value_len: u32 | body_crc: u32 | key | value
//! ```
//!
//! All integers are little-endian. `header_crc` covers the two lengths and is checked before they
//! are trusted. `body_crc` covers the key and the value. A deletion stores `value_len = TOMBSTONE`
//! and no value bytes.

use std::io::{self, Read};

use thiserror::Error;

/// Bytes before the key: four `u32` fields.
pub const HEADER_LEN: usize = 16;

/// The `value_len` that marks a deletion. No real value can be this long, because encoding refuses it.
pub const TOMBSTONE: u32 = u32::MAX;

/// One entry in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// Set `key` to `value`.
    Put { key: Vec<u8>, value: Vec<u8> },
    /// Delete `key`.
    Delete { key: Vec<u8> },
}

impl Record {
    /// The key this record is about.
    #[must_use]
    pub fn key(&self) -> &[u8] {
        match self {
            Record::Put { key, .. } | Record::Delete { key } => key,
        }
    }
}

/// Size limits, checked when encoding and again when decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Longest key allowed, in bytes.
    pub max_key: u32,
    /// Longest value allowed, in bytes.
    pub max_value: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_key: 1024,
            max_value: 1024 * 1024,
        }
    }
}

/// Why a record could not be encoded.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EncodeError {
    /// Keys must have at least one byte.
    #[error("key is empty")]
    EmptyKey,
    /// The key is longer than `Limits::max_key`.
    #[error("key is {len} bytes, above the limit of {max}")]
    KeyTooLong { len: usize, max: u32 },
    /// The value is longer than `Limits::max_value`.
    #[error("value is {len} bytes, above the limit of {max}")]
    ValueTooLong { len: usize, max: u32 },
}

/// Appends the encoded form of `record` to `out`.
///
/// # Errors
///
/// Returns an error, and leaves `out` unchanged, if the key is empty or a key or value is longer
/// than `limits` allows.
pub fn encode(record: &Record, limits: Limits, out: &mut Vec<u8>) -> Result<(), EncodeError> {
    let key = record.key();
    if key.is_empty() {
        return Err(EncodeError::EmptyKey);
    }
    let key_len = checked_len(key.len(), limits.max_key).ok_or(EncodeError::KeyTooLong {
        len: key.len(),
        max: limits.max_key,
    })?;

    let (value, value_len) = match record {
        Record::Put { value, .. } => {
            let len = checked_len(value.len(), limits.max_value)
                .filter(|&len| len != TOMBSTONE)
                .ok_or(EncodeError::ValueTooLong {
                    len: value.len(),
                    max: limits.max_value,
                })?;
            (value.as_slice(), len)
        }
        Record::Delete { .. } => (&[][..], TOMBSTONE),
    };

    let mut lengths = [0u8; 8];
    lengths[..4].copy_from_slice(&key_len.to_le_bytes());
    lengths[4..].copy_from_slice(&value_len.to_le_bytes());
    let header_crc = crc32c::crc32c(&lengths);
    let body_crc = crc32c::crc32c_append(crc32c::crc32c(key), value);

    out.reserve(HEADER_LEN + key.len() + value.len());
    out.extend_from_slice(&header_crc.to_le_bytes());
    out.extend_from_slice(&lengths);
    out.extend_from_slice(&body_crc.to_le_bytes());
    out.extend_from_slice(key);
    out.extend_from_slice(value);
    Ok(())
}

/// `len` as a `u32`, or `None` if it does not fit or is above `max`.
fn checked_len(len: usize, max: u32) -> Option<u32> {
    u32::try_from(len).ok().filter(|&len| len <= max)
}

/// What [`decode`] found at one position in the log.
#[derive(Debug, PartialEq, Eq)]
pub enum Decoded {
    /// A complete, intact record that took `len` bytes.
    Record { record: Record, len: u64 },
    /// No bytes left: the log ends cleanly here.
    CleanEnd,
    /// The log ends part-way through a record, which is what a crash mid-write leaves behind. That
    /// record was never acknowledged. Everything before `valid_bytes` is intact.
    TornTail { valid_bytes: u64 },
    /// A complete record failed a check. Its contents cannot be trusted, so recovery must stop.
    Corrupt { offset: u64, reason: CorruptReason },
}

/// Which check a corrupt record failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorruptReason {
    /// The lengths do not match `header_crc`, so they cannot be trusted.
    BadHeader,
    /// A checked header says the key is empty, which `encode` never writes.
    EmptyKey,
    /// A checked header says the key is longer than the limit.
    KeyTooLong,
    /// A checked header says the value is longer than the limit.
    ValueTooLong,
    /// The key and value do not match `body_crc`.
    BadBody,
}

/// Reads one record from `reader`, which is positioned `offset` bytes into the log.
///
/// Every length is checked against its checksum and against `limits` before any memory is
/// allocated for it, so damaged or hostile input can never cause a huge allocation.
///
/// # Errors
///
/// Returns an error only if reading itself fails. Damaged data is never an error: it is reported as
/// [`Decoded::TornTail`] or [`Decoded::Corrupt`].
pub fn decode(reader: &mut impl Read, offset: u64, limits: Limits) -> io::Result<Decoded> {
    let corrupt =
        |reason: CorruptReason| -> io::Result<Decoded> { Ok(Decoded::Corrupt { offset, reason }) };

    let mut header = [0u8; HEADER_LEN];
    let got = read_up_to(reader, &mut header)?;
    if got == 0 {
        return Ok(Decoded::CleanEnd);
    }
    if got < HEADER_LEN {
        return Ok(Decoded::TornTail {
            valid_bytes: offset,
        });
    }

    // 1. Check: the lengths must match their own checksum before they are believed.
    if crc32c::crc32c(&header[4..12]) != u32_at(&header, 0) {
        return corrupt(CorruptReason::BadHeader);
    }
    let key_len = u32_at(&header, 4);
    let value_len = u32_at(&header, 8);
    let body_crc = u32_at(&header, 12);

    // 2. Check: the believed lengths must be within the limits, still before allocating.
    if key_len == 0 {
        return corrupt(CorruptReason::EmptyKey);
    }
    if key_len > limits.max_key {
        return corrupt(CorruptReason::KeyTooLong);
    }
    let is_delete = value_len == TOMBSTONE;
    if !is_delete && value_len > limits.max_value {
        return corrupt(CorruptReason::ValueTooLong);
    }

    // 3. Trust: allocate exactly what the checked lengths say. Running out of bytes now really
    //    means the crash interrupted this record.
    let key_len = key_len as usize;
    let value_len = if is_delete { 0 } else { value_len as usize };
    let mut body = vec![0u8; key_len + value_len];
    if read_up_to(reader, &mut body)? < body.len() {
        return Ok(Decoded::TornTail {
            valid_bytes: offset,
        });
    }
    if crc32c::crc32c(&body) != body_crc {
        return corrupt(CorruptReason::BadBody);
    }

    let len = (HEADER_LEN + body.len()) as u64;
    let value = body.split_off(key_len); // body keeps the key, value takes the rest
    let record = if is_delete {
        Record::Delete { key: body }
    } else {
        Record::Put { key: body, value }
    };
    Ok(Decoded::Record { record, len })
}

/// Fills `buf` as far as the reader allows and returns how many bytes it got. Unlike a single
/// `read`, it only stops early at the end of the input.
fn read_up_to(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// The little-endian `u32` at `bytes[at..at + 4]`.
fn u32_at(bytes: &[u8; HEADER_LEN], at: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[at..at + 4]);
    u32::from_le_bytes(word)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn put(key: &[u8], value: &[u8]) -> Record {
        Record::Put {
            key: key.to_vec(),
            value: value.to_vec(),
        }
    }

    fn u32_at_slice(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn put_has_the_documented_layout() {
        let mut out = Vec::new();
        encode(&put(b"k", b"value"), Limits::default(), &mut out).unwrap();

        assert_eq!(out.len(), HEADER_LEN + 1 + 5);
        assert_eq!(u32_at_slice(&out, 4), 1, "key_len");
        assert_eq!(u32_at_slice(&out, 8), 5, "value_len");
        assert_eq!(
            u32_at_slice(&out, 0),
            crc32c::crc32c(&out[4..12]),
            "header_crc covers the lengths"
        );
        assert_eq!(
            u32_at_slice(&out, 12),
            crc32c::crc32c(b"kvalue"),
            "body_crc covers key and value"
        );
        assert_eq!(&out[16..], b"kvalue");
    }

    #[test]
    fn delete_stores_a_tombstone_and_no_value() {
        let mut out = Vec::new();
        let record = Record::Delete { key: b"k".to_vec() };
        encode(&record, Limits::default(), &mut out).unwrap();

        assert_eq!(out.len(), HEADER_LEN + 1);
        assert_eq!(u32_at_slice(&out, 8), TOMBSTONE);
    }

    #[test]
    fn empty_key_is_refused_and_nothing_is_written() {
        let mut out = vec![7];
        let result = encode(&put(b"", b"v"), Limits::default(), &mut out);

        assert_eq!(result, Err(EncodeError::EmptyKey));
        assert_eq!(out, vec![7], "out must be unchanged");
    }

    #[test]
    fn lengths_above_the_limits_are_refused() {
        let limits = Limits {
            max_key: 2,
            max_value: 3,
        };
        let mut out = Vec::new();

        assert_eq!(
            encode(&put(b"abc", b"v"), limits, &mut out),
            Err(EncodeError::KeyTooLong { len: 3, max: 2 })
        );
        assert_eq!(
            encode(&put(b"k", b"abcd"), limits, &mut out),
            Err(EncodeError::ValueTooLong { len: 4, max: 3 })
        );
        assert_eq!(out, Vec::<u8>::new(), "nothing written");
    }

    fn encoded(record: &Record) -> Vec<u8> {
        let mut out = Vec::new();
        encode(record, Limits::default(), &mut out).unwrap();
        out
    }

    fn decode_all(bytes: &[u8]) -> Decoded {
        decode(&mut Cursor::new(bytes), 0, Limits::default()).unwrap()
    }

    /// A header with the given lengths and a correct `header_crc`, as damage could produce.
    fn header_claiming(key_len: u32, value_len: u32) -> Vec<u8> {
        let mut lengths = Vec::new();
        lengths.extend_from_slice(&key_len.to_le_bytes());
        lengths.extend_from_slice(&value_len.to_le_bytes());
        let mut out = crc32c::crc32c(&lengths).to_le_bytes().to_vec();
        out.extend_from_slice(&lengths);
        out.extend_from_slice(&[0; 4]);
        out
    }

    #[test]
    fn put_and_delete_round_trip() {
        for record in [
            put(b"key", b"value"),
            Record::Delete {
                key: b"key".to_vec(),
            },
        ] {
            let bytes = encoded(&record);
            let len = bytes.len() as u64;
            assert_eq!(decode_all(&bytes), Decoded::Record { record, len });
        }
    }

    #[test]
    fn consecutive_records_then_a_clean_end() {
        let mut log = encoded(&put(b"a", b"1"));
        log.extend(encoded(&put(b"b", b"22")));
        let mut reader = Cursor::new(log);

        let first = decode(&mut reader, 0, Limits::default()).unwrap();
        assert_eq!(
            first,
            Decoded::Record {
                record: put(b"a", b"1"),
                len: 18
            }
        );
        let second = decode(&mut reader, 18, Limits::default()).unwrap();
        assert_eq!(
            second,
            Decoded::Record {
                record: put(b"b", b"22"),
                len: 19
            }
        );
        assert_eq!(
            decode(&mut reader, 37, Limits::default()).unwrap(),
            Decoded::CleanEnd
        );
    }

    #[test]
    fn every_truncation_point_is_a_torn_tail() {
        let bytes = encoded(&put(b"key", b"value"));
        for cut in 1..bytes.len() {
            assert_eq!(
                decode_all(&bytes[..cut]),
                Decoded::TornTail { valid_bytes: 0 },
                "cut after {cut} bytes"
            );
        }
    }

    #[test]
    fn every_single_bit_flip_is_detected() {
        let bytes = encoded(&put(b"key", b"value"));
        for bit in 0..bytes.len() * 8 {
            let mut damaged = bytes.clone();
            damaged[bit / 8] ^= 1 << (bit % 8);
            assert!(
                matches!(decode_all(&damaged), Decoded::Corrupt { .. }),
                "flipping bit {bit} was not reported as corrupt"
            );
        }
    }

    #[test]
    fn huge_lengths_are_refused_before_allocating() {
        // Valid header checksums, so only the limit check stands between these and a 4 GB allocation.
        assert_eq!(
            decode_all(&header_claiming(4_000_000_000, 1)),
            Decoded::Corrupt {
                offset: 0,
                reason: CorruptReason::KeyTooLong
            }
        );
        assert_eq!(
            decode_all(&header_claiming(1, 4_000_000_000)),
            Decoded::Corrupt {
                offset: 0,
                reason: CorruptReason::ValueTooLong
            }
        );
        assert_eq!(
            decode_all(&header_claiming(0, 1)),
            Decoded::Corrupt {
                offset: 0,
                reason: CorruptReason::EmptyKey
            }
        );
    }
}
