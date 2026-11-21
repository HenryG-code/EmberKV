use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::time::Instant;                              // NEW: a high-resolution stopwatch

fn append_record(file: &mut File, n: u64, sync: bool) -> io::Result<()> {
    let payload = n.to_string().into_bytes();
    // `as u32` would silently keep only the low 32 bits of a huge length, and the record would still
    // look valid. try_from fails instead, so a length that does not fit can never be written.
    let len = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload longer than u32::MAX"))?;
    let len_bytes = len.to_le_bytes();

    let header_crc = crc32c::crc32c(&len_bytes);
    let body_crc = crc32c::crc32c(&payload);

    let mut record = Vec::with_capacity(12 + payload.len());
    record.extend_from_slice(&header_crc.to_le_bytes());
    record.extend_from_slice(&len_bytes);
    record.extend_from_slice(&body_crc.to_le_bytes());
    record.extend_from_slice(&payload);

    file.write_all(&record)?;
    if sync {
        file.sync_all()?;
    }
    Ok(())
}

fn main() -> io::Result<()> {
    // NEW: collect all command-line words into a Vec, so we can look at them more than once.
    let args: Vec<String> = std::env::args().collect();

    let count: u64 = args
        .get(1)                                          // Option<&String>: maybe missing
        .map(|s| s.parse().expect("count must be a number")) // NEW: closure, |s| ... is a tiny inline function
        .unwrap_or(5);

    let sync = !args.iter().any(|a| a == "--no-sync"); // NEW: iterator + any()
    let quiet = args.iter().any(|a| a == "--quiet");

    let mut file = OpenOptions::new().create(true).append(true).open("data.log")?;
    // sync_all on the file makes its contents durable, but the file's NAME lives in the folder.
    // Flush the folder too, so a newly created data.log cannot disappear after a power cut.
    File::open(".")?.sync_all()?;

    let start = Instant::now();
    for n in 1..=count {
        append_record(&mut file, n, sync)?;
        if !quiet {
            println!("{n}");
        }
    }
    let secs = start.elapsed().as_secs_f64();           // NEW: Duration → seconds as a decimal

    eprintln!(
        "{count} records in {secs:.3}s = {:.0} records/s (sync every write: {sync})",
        count as f64 / secs                              // NEW: `as f64` converts an integer to a decimal
    );
    Ok(())
}