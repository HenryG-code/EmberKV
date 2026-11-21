use std::fs::File;
use std::io::{self, BufReader, Read};

enum Decoded {
    Record { n: u64, len: u64 },
    CleanEnd,
    TornTail,
    Corrupt { offset: u64 },
}

// NEW: fill `buf` as far as possible and report how many bytes we really got.
// A plain `read` may return fewer bytes than asked even when more exist,
// so we loop until the buffer is full or the file ends (read returns 0).
fn read_up_to(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {      // read into the part not yet filled
            Ok(0) => break,                           // end of file
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue, // NEW: a "match guard"
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

fn read_record(reader: &mut impl Read, offset: u64) -> io::Result<Decoded> {
    let mut header = [0u8; 8];
    let got = read_up_to(reader, &mut header)?;
    if got == 0 { return Ok(Decoded::CleanEnd); }
    if got < 8  { return Ok(Decoded::TornTail); }

    let checksum = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let len      = u32::from_le_bytes(header[4..8].try_into().unwrap());
    if len > 1024 { return Ok(Decoded::Corrupt { offset }); }

    let mut payload = vec![0u8; len as usize];
    if read_up_to(reader, &mut payload)? < len as usize {
        return Ok(Decoded::TornTail);
    }
    if crc32c::crc32c(&payload) != checksum {
        return Ok(Decoded::Corrupt { offset });
    }

    let text = String::from_utf8(payload).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    let n: u64 = text.parse().map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    Ok(Decoded::Record { n, len: 8 + len as u64 })
}

fn main() -> io::Result<()> {
    let path = std::env::args().nth(1).unwrap_or("data.log".to_string());
    let mut reader = BufReader::new(File::open(&path)?);   // NEW: BufReader

    let mut offset: u64 = 0;
    let mut records: u64 = 0;
    let mut last_good: Option<u64> = None;                  // NEW: Option as a variable

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
                println!("RECOVERED: {records} good records, last {last_good:?}, \
                          discarded a torn tail at byte {offset}");
                return Ok(());
            }
            Decoded::Corrupt { offset } => {
                eprintln!("CORRUPT: damaged record at byte {offset}, refusing to continue");
                std::process::exit(1);                     // NEW: non-zero exit = failure
            }
        }
    }
}
