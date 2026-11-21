use std::fs::File;
use std::io::{self, BufReader, Read};

// ---------- Types: what a record can turn out to be ----------

#[derive(Debug)]
enum CorruptReason {
    BadHeader, // the lengths don't match their checksum
    TooLong,   // a checked length that is still above our limit
    BadBody,   // the payload doesn't match its checksum
}

enum Decoded {
    Record { n: u64, len: u64 },
    CleanEnd,
    TornTail,
    Corrupt { offset: u64, reason: CorruptReason },
}

// ---------- The 12-byte header ----------

struct Header {
    header_crc: u32,
    len: u32,
    body_crc: u32,
}

impl Header {
    const SIZE: usize = 12;

    fn parse(bytes: &[u8; Header::SIZE]) -> Header {
        Header {
            header_crc: u32_at(bytes, 0),
            len: u32_at(bytes, 4),
            body_crc: u32_at(bytes, 8),
        }
    }

    fn is_valid(&self) -> bool {
        crc32c::crc32c(&self.len.to_le_bytes()) == self.header_crc
    }
}

// ---------- Small helpers ----------

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn read_up_to(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

// ---------- Reading one record: check, then trust ----------

fn read_record(reader: &mut impl Read, offset: u64) -> io::Result<Decoded> {
    let mut buf = [0u8; Header::SIZE];
    let got = read_up_to(reader, &mut buf)?;
    if got == 0 {
        return Ok(Decoded::CleanEnd);
    }
    if got < Header::SIZE {
        return Ok(Decoded::TornTail);
    }

    let header = Header::parse(&buf);

    // Verify the length before using it for anything.
    if !header.is_valid() {
        return Ok(Decoded::Corrupt { offset, reason: CorruptReason::BadHeader });
    }
    if header.len > 1024 {
        return Ok(Decoded::Corrupt { offset, reason: CorruptReason::TooLong });
    }

    // From here on, the length is trusted, so running out of bytes really means a torn write.
    let mut payload = vec![0u8; header.len as usize];
    if read_up_to(reader, &mut payload)? < header.len as usize {
        return Ok(Decoded::TornTail);
    }
    if crc32c::crc32c(&payload) != header.body_crc {
        return Ok(Decoded::Corrupt { offset, reason: CorruptReason::BadBody });
    }

    let text = String::from_utf8(payload).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    let n: u64 = text.parse().map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    Ok(Decoded::Record { n, len: Header::SIZE as u64 + header.len as u64 })
}

// ---------- The program ----------

fn main() -> io::Result<()> {
    let path = std::env::args().nth(1).unwrap_or("data.log".to_string());
    let mut reader = BufReader::new(File::open(&path)?);

    let mut offset: u64 = 0;
    let mut records: u64 = 0;
    let mut last_good: Option<u64> = None;

    loop {
        match read_record(&mut reader, offset)? {
            Decoded::Record { n, len } => {
                records += 1;
                last_good = Some(n);
                offset += len;
            }
            Decoded::CleanEnd => {
                println!("OK: {records} records, last {last_good:?}, file is clean");
                return Ok(());
            }
            Decoded::TornTail => {
                println!(
                    "RECOVERED: {records} good records, last {last_good:?}, \
                     discarded a torn tail at byte {offset}"
                );
                return Ok(());
            }
            Decoded::Corrupt { offset, reason } => {
                eprintln!("CORRUPT: {reason:?} at byte {offset}, refusing to continue");
                std::process::exit(1);
            }
        }
    }
}