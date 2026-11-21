use std::fs::{File, OpenOptions};
use std::io::{self, Write};

fn append_record(file: &mut File, n: u64) -> io::Result<()> {
    let payload = n.to_string().into_bytes();
    let checksum = crc32c::crc32c(&payload);

    let mut record = Vec::new();
    record.extend_from_slice(&checksum.to_le_bytes());
    record.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    record.extend_from_slice(&payload);

    file.write_all(&record)?;
    file.sync_all()?;
    Ok(())
}


fn  main() -> io::Result<()> {
    let count:u64 = std::env::args()// the words typed after the program name
        .nth(1)// get the first argument
        .expect("Please provide a count")// if there is no argument, print this message
        .parse()// parse the argument as a u64
        .expect("Please provide a valid number");// if the argument is not a valid number, print this message

        let mut file = OpenOptions::new().create(true).append(true).open("data.log")?;
    for n in 1..=count {
        append_record(&mut file, n)?;
        println!("{n}");
    }
    Ok(())
}
