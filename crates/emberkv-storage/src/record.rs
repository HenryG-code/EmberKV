//! The on-disk record format, as decided in ADR 0002.
//!
//! ```text
//! header_crc: u32 | key_len: u32 | value_len: u32 | body_crc: u32 | key | value
//! ```
//!
//! All integers are little-endian. `header_crc` covers the two lengths and is checked before they
//! are trusted. `body_crc` covers the key and the value. A deletion stores `value_len = TOMBSTONE`
//! and no value bytes.

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

#[cfg(test)]
mod tests {
    use super::*;

    fn put(key: &[u8], value: &[u8]) -> Record {
        Record::Put {
            key: key.to_vec(),
            value: value.to_vec(),
        }
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn put_has_the_documented_layout() {
        let mut out = Vec::new();
        encode(&put(b"k", b"value"), Limits::default(), &mut out).unwrap();

        assert_eq!(out.len(), HEADER_LEN + 1 + 5);
        assert_eq!(u32_at(&out, 4), 1, "key_len");
        assert_eq!(u32_at(&out, 8), 5, "value_len");
        assert_eq!(
            u32_at(&out, 0),
            crc32c::crc32c(&out[4..12]),
            "header_crc covers the lengths"
        );
        assert_eq!(
            u32_at(&out, 12),
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
        assert_eq!(u32_at(&out, 8), TOMBSTONE);
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
}