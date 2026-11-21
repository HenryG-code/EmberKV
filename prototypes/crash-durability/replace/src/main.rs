use std::fs::{self, File};
use std::io::{self, Write};

const LINES: usize = 200_000;   // NEW: a constant outside any type. `_` in numbers is just for reading.

// Write 200,000 identical lines like "generation 7", then flush.
// Writing one line at a time is slow on purpose, so the kill lands mid-write.
fn write_content(file: &mut File, generation: u64) -> io::Result<()> {
    let line = format!("generation {generation}\n");   // NEW: format! builds a String, like println! without printing
    for _ in 0..LINES {                                 // NEW: `_` = we don't need the loop counter
        file.write_all(line.as_bytes())?;
    }
    file.sync_all()
}

// The naive way: File::create wipes "data" immediately, then refills it.
fn unsafe_replace(generation: u64) -> io::Result<()> {
    let mut file = File::create("data")?;
    write_content(&mut file, generation)
}

// The safe way: build the new copy beside the old one, then swap.
fn safe_replace(generation: u64) -> io::Result<()> {
    let mut tmp = File::create("data.tmp")?;
    write_content(&mut tmp, generation)?;   // 1 + 2: complete new copy, flushed
    fs::rename("data.tmp", "data")?;        // 3: atomic swap of the name
    File::open(".")?.sync_all()             // 4: flush the folder, so the swap is on disk
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("safe");   // NEW: String → &str
    let generation: u64 = args
        .get(2)
        .map(|s| s.parse().expect("generation must be a number"))
        .unwrap_or(1);

    match mode {                                   // NEW: match on text
        "safe" => safe_replace(generation),
        "unsafe" => unsafe_replace(generation),
        other => {
            eprintln!("unknown mode '{other}': use safe or unsafe");
            std::process::exit(2);
        }
    }
}
