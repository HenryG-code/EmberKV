use std::fs::{File, OpenOptions};
use std::io::{self, Write};

fn append_record(file: &mut File, n: u64) -> io::Result<()> {
    let payload = n.to_string().into_bytes();
    let len_bytes = (payload.len() as u32).to_le_bytes();

    let header_crc = crc32c::crc32c(&len_bytes);
    let body_crc   = crc32c::crc32c(&payload);

    let mut record = Vec::with_capacity(12 + payload.len());
    record.extend_from_slice(&header_crc.to_le_bytes());
    record.extend_from_slice(&len_bytes);
    record.extend_from_slice(&body_crc.to_le_bytes());
    record.extend_from_slice(&payload);

    file.write_all(&record)?;
    file.sync_all()?;
    Ok(())
}

fn main() -> io::Result<()> {
    let count: u64 = std::env::args()
        .nth(1)
        .unwrap_or("5".to_string())
        .parse()
        .expect("count must be a number");

    let mut file = OpenOptions::new().create(true).append(true).open("data.log")?;
    for n in 1..=count {
        append_record(&mut file, n)?;
        println!("{n}");
    }
    Ok(())
}