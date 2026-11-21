//! Recovery: on startup, read every segment from the beginning and decide what can be trusted.
//!
//! The rules, from ADR 0002 and the architecture:
//!
//! - Segments must be numbered without gaps; a missing file means lost data.
//! - Only the **newest** segment may end in a torn tail, because a crash can only interrupt the
//!   last write. The tail is cut off and the cut is flushed.
//! - A torn tail is accepted only if no valid record can be found after it. A valid record there
//!   proves the damage came from somewhere else, not from a crash.
//! - Any corrupt record stops recovery. Nothing is skipped, and nothing is guessed.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Cursor};
use std::path::Path;

use thiserror::Error;

use crate::record::{CorruptReason, Decoded, HEADER_LEN, Limits, Record, decode};
use crate::segment::{Location, parse_segment_file_name, segment_file_name};

/// Why recovery refused to continue. Every variant except `Io` means the data folder needs a person
/// to look at it.
#[derive(Debug, Error)]
pub enum RecoveryError {
    /// The folder or a segment could not be read, or the torn tail could not be cut off.
    #[error("disk error during recovery: {0}")]
    Io(#[from] io::Error),
    /// Segment numbers have a gap, so a whole file of records is missing.
    #[error("segment {expected:06} is missing")]
    MissingSegment { expected: u32 },
    /// A complete record failed a check.
    #[error("segment {segment:06} is corrupt at byte {offset}: {reason:?}")]
    Corrupt {
        segment: u32,
        offset: u64,
        reason: CorruptReason,
    },
    /// A segment that is not the newest ends part-way through a record, which no crash can cause.
    #[error(
        "segment {segment:06} ends part-way through a record at byte {offset}, but it is not the newest segment"
    )]
    TornOlderSegment { segment: u32, offset: u64 },
    /// The newest segment looks torn at `offset`, but a valid record exists after that point.
    #[error(
        "segment {segment:06} looks torn at byte {offset}, but valid data follows; refusing to cut it"
    )]
    DataAfterTear { segment: u32, offset: u64 },
}

/// What recovery found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Segment numbers, oldest first.
    pub segments: Vec<u32>,
    /// Valid records read across all segments.
    pub records: u64,
    /// Bytes cut off the end of the newest segment, if it ended in a torn tail.
    pub discarded_bytes: u64,
    /// Size of the newest segment after recovery, where the next record will be appended.
    pub newest_size: u64,
}

/// Reads every segment in `dir`, oldest first, and calls `on_record` for each valid record in order.
///
/// A torn tail at the end of the newest segment is cut off and the cut is flushed, so calling this
/// again finds a clean log.
///
/// # Errors
///
/// Returns a [`RecoveryError`] if a segment is missing, corrupt, torn anywhere except the end of the
/// newest segment, or if the disk fails. Nothing is changed on disk in those cases.
pub fn recover(
    dir: &Path,
    limits: Limits,
    mut on_record: impl FnMut(Record, Location),
) -> Result<RecoveryReport, RecoveryError> {
    let segments = list_segments(dir)?;
    let mut report = RecoveryReport {
        segments: segments.clone(),
        ..RecoveryReport::default()
    };

    for (position, &segment) in segments.iter().enumerate() {
        let is_newest = position + 1 == segments.len();
        let path = dir.join(segment_file_name(segment));
        let mut reader = BufReader::new(File::open(&path)?);
        let mut offset = 0;

        loop {
            match decode(&mut reader, offset, limits)? {
                Decoded::Record { record, len } => {
                    on_record(
                        record,
                        Location {
                            segment,
                            offset,
                            len,
                        },
                    );
                    report.records += 1;
                    offset += len;
                }
                Decoded::CleanEnd => break,
                Decoded::Corrupt { offset, reason } => {
                    return Err(RecoveryError::Corrupt {
                        segment,
                        offset,
                        reason,
                    });
                }
                Decoded::TornTail { valid_bytes } => {
                    if !is_newest {
                        return Err(RecoveryError::TornOlderSegment {
                            segment,
                            offset: valid_bytes,
                        });
                    }
                    report.discarded_bytes = cut_torn_tail(&path, segment, valid_bytes, limits)?;
                    break;
                }
            }
        }
        if is_newest {
            report.newest_size = offset;
        }
    }
    Ok(report)
}

/// Segment numbers in `dir`, sorted, after checking there are no gaps.
fn list_segments(dir: &Path) -> Result<Vec<u32>, RecoveryError> {
    let mut segments = Vec::new();
    for entry in fs::read_dir(dir)? {
        let name = entry?.file_name();
        if let Some(id) = name.to_str().and_then(parse_segment_file_name) {
            segments.push(id);
        }
    }
    segments.sort_unstable();
    for pair in segments.windows(2) {
        if pair[1] != pair[0] + 1 {
            return Err(RecoveryError::MissingSegment {
                expected: pair[0] + 1,
            });
        }
    }
    Ok(segments)
}

/// Cuts the newest segment back to `valid_bytes`, after making sure no valid record follows the tear.
/// Returns how many bytes were cut.
fn cut_torn_tail(
    path: &Path,
    segment: u32,
    valid_bytes: u64,
    limits: Limits,
) -> Result<u64, RecoveryError> {
    let bytes = fs::read(path)?;
    let start = usize::try_from(valid_bytes).map_err(io::Error::other)?;

    // Skip the torn record's own header: its body may legitimately hold any bytes.
    for position in (start + HEADER_LEN)..bytes.len() {
        let mut reader = Cursor::new(&bytes[position..]);
        if let Decoded::Record { .. } = decode(&mut reader, 0, limits)? {
            return Err(RecoveryError::DataAfterTear {
                segment,
                offset: valid_bytes,
            });
        }
    }

    let file = OpenOptions::new().write(true).open(path)?;
    file.set_len(valid_bytes)?;
    file.sync_all()?;
    Ok(bytes.len() as u64 - valid_bytes)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::record::encode;
    use crate::segment::SegmentWriter;

    fn put(key: &[u8], value: &[u8]) -> Record {
        Record::Put {
            key: key.to_vec(),
            value: value.to_vec(),
        }
    }

    /// Writes one segment holding `records`, flushed, and returns its path.
    fn write_segment(dir: &Path, id: u32, records: &[Record]) -> PathBuf {
        let mut writer = SegmentWriter::create(dir, id, Limits::default()).unwrap();
        for record in records {
            writer.append(record).unwrap();
        }
        writer.sync().unwrap();
        dir.join(segment_file_name(id))
    }

    /// Runs recovery and collects every record it reports.
    fn recover_all(dir: &Path) -> Result<(RecoveryReport, Vec<(Record, Location)>), RecoveryError> {
        let mut seen = Vec::new();
        let report = recover(dir, Limits::default(), |record, location| {
            seen.push((record, location));
        })?;
        Ok((report, seen))
    }

    fn file_len(path: &Path) -> u64 {
        fs::metadata(path).unwrap().len()
    }

    #[test]
    fn an_empty_folder_recovers_to_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (report, seen) = recover_all(dir.path()).unwrap();
        assert_eq!(report, RecoveryReport::default());
        assert_eq!(seen, Vec::new(), "no records");
    }

    #[test]
    fn records_come_back_in_order_with_their_locations() {
        let dir = tempfile::tempdir().unwrap();
        write_segment(dir.path(), 1, &[put(b"a", b"1"), put(b"b", b"22")]);
        write_segment(dir.path(), 2, &[Record::Delete { key: b"a".to_vec() }]);
        fs::write(dir.path().join("LOCK"), b"").unwrap(); // other files are ignored
        fs::write(dir.path().join("000003.log.tmp"), b"junk").unwrap();

        let (report, seen) = recover_all(dir.path()).unwrap();

        assert_eq!(report.segments, vec![1, 2]);
        assert_eq!(report.records, 3);
        assert_eq!(report.newest_size, 17);
        let locations: Vec<Location> = seen.iter().map(|(_, location)| *location).collect();
        assert_eq!(
            locations,
            vec![
                Location {
                    segment: 1,
                    offset: 0,
                    len: 18
                },
                Location {
                    segment: 1,
                    offset: 18,
                    len: 19
                },
                Location {
                    segment: 2,
                    offset: 0,
                    len: 17
                },
            ]
        );
    }

    #[test]
    fn a_torn_tail_in_the_newest_segment_is_cut_off_and_stays_cut() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_segment(dir.path(), 1, &[put(b"a", b"1"), put(b"b", b"22")]);
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(file_len(&path) - 3).unwrap(); // the crash cut the last record short

        let (report, seen) = recover_all(dir.path()).unwrap();
        assert_eq!(report.records, 1);
        assert_eq!(report.discarded_bytes, 16);
        assert_eq!(report.newest_size, 18);
        assert_eq!(seen.len(), 1);
        assert_eq!(file_len(&path), 18, "the tail was cut on disk");

        let (again, _) = recover_all(dir.path()).unwrap();
        assert_eq!(
            again.discarded_bytes, 0,
            "a second recovery finds a clean log"
        );
    }

    #[test]
    fn a_torn_older_segment_stops_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let older = write_segment(dir.path(), 1, &[put(b"a", b"1")]);
        write_segment(dir.path(), 2, &[put(b"b", b"2")]);
        let file = OpenOptions::new().write(true).open(&older).unwrap();
        file.set_len(10).unwrap();

        let error = recover_all(dir.path()).unwrap_err();
        assert!(matches!(
            error,
            RecoveryError::TornOlderSegment {
                segment: 1,
                offset: 0
            }
        ));
        assert_eq!(file_len(&older), 10, "nothing is changed on disk");
    }

    #[test]
    fn a_flipped_bit_stops_recovery_and_names_the_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_segment(dir.path(), 1, &[put(b"a", b"1"), put(b"b", b"22")]);
        let mut bytes = fs::read(&path).unwrap();
        bytes[18 + 16] ^= 0x01; // the key byte of the second record
        fs::write(&path, &bytes).unwrap();

        let error = recover_all(dir.path()).unwrap_err();
        assert!(matches!(
            error,
            RecoveryError::Corrupt {
                segment: 1,
                offset: 18,
                reason: CorruptReason::BadBody
            }
        ));
    }

    #[test]
    fn a_missing_segment_stops_recovery() {
        let dir = tempfile::tempdir().unwrap();
        write_segment(dir.path(), 1, &[put(b"a", b"1")]);
        write_segment(dir.path(), 3, &[put(b"c", b"3")]);

        let error = recover_all(dir.path()).unwrap_err();
        assert!(matches!(
            error,
            RecoveryError::MissingSegment { expected: 2 }
        ));
    }

    #[test]
    fn a_tear_with_valid_data_after_it_is_refused_not_cut() {
        // Lesson 4's bug, the rare form: a damaged length that still matches its header checksum
        // makes a record look torn, yet a good record follows it.
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = Vec::new();
        encode(&put(b"a", b"1"), Limits::default(), &mut bytes).unwrap();
        let lengths = [1u32.to_le_bytes(), 1000u32.to_le_bytes()].concat();
        bytes.extend_from_slice(&crc32c::crc32c(&lengths).to_le_bytes());
        bytes.extend_from_slice(&lengths);
        bytes.extend_from_slice(&[0; 4]);
        encode(&put(b"c", b"3"), Limits::default(), &mut bytes).unwrap();
        let path = dir.path().join(segment_file_name(1));
        fs::write(&path, &bytes).unwrap();

        let error = recover_all(dir.path()).unwrap_err();
        assert!(matches!(
            error,
            RecoveryError::DataAfterTear {
                segment: 1,
                offset: 18
            }
        ));
        assert_eq!(file_len(&path), bytes.len() as u64, "nothing was cut");
    }
}
