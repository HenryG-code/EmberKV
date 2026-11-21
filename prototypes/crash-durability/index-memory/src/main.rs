use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::hint::black_box;
use std::mem::size_of;

// Where a key's newest record lives on disk: the value stored in the index for every key.
#[derive(Clone, Copy)]
struct Location {
    segment: u32, // which log file
    offset: u64,  // byte position inside it
    len: u32,     // record size
}

// NEW: a "tuple struct": a struct with one unnamed field, accessed as self.0
struct Rng(u64);

impl Rng {
    // xorshift64: a tiny, fast, NOT cryptographic random generator. Fine for test keys.
    fn next(&mut self) -> u64 {     // `&mut self`: this method changes the generator's state
        let mut x = self.0;
        x ^= x << 13;               // NEW: ^ is XOR, << shifts bits left
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn key(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next() as u8).collect()   // NEW: build a Vec from an iterator
    }
}

// Read this process's current memory use, in kilobytes, from Linux's /proc.
fn rss_kb() -> u64 {
    let status = fs::read_to_string("/proc/self/status").expect("this needs Linux");
    status
        .lines()                                        // one line at a time
        .find(|line| line.starts_with("VmRSS:"))        // e.g. "VmRSS:     1932 kB"
        .and_then(|line| line.split_whitespace().nth(1))// NEW: and_then chains Option steps
        .and_then(|number| number.parse().ok())         // NEW: .ok() turns Result into Option
        .expect("VmRSS line not found")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let count: usize = args.get(1).map(|s| s.parse().expect("count")).unwrap_or(1_000_000);
    let key_len: usize = args.get(2).map(|s| s.parse().expect("key length")).unwrap_or(16);
    let kind = args.get(3).map(String::as_str).unwrap_or("btree");

    let mut rng = Rng(0x2545_F491_4F6C_DD1D);           // fixed seed: same keys every run
    let location = Location { segment: 1, offset: 0, len: 0 };

    let before = rss_kb();
    let entries = match kind {
        "btree" => {
            let mut map = BTreeMap::new();
            for i in 0..count {
                map.insert(rng.key(key_len), Location { offset: i as u64, ..location }); // NEW: ..location
            }
            black_box(&map);                            // NEW: stop the optimiser removing the map
            let after = rss_kb();
            report("BTreeMap", count, key_len, before, after);
            map.len()
        }
        "hash" => {
            let mut map = HashMap::new();
            for i in 0..count {
                map.insert(rng.key(key_len), Location { offset: i as u64, ..location });
            }
            black_box(&map);
            let after = rss_kb();
            report("HashMap", count, key_len, before, after);
            map.len()
        }
        other => panic!("unknown map kind '{other}': use btree or hash"),
    };
    println!("entries kept: {entries}");
}

fn report(name: &str, count: usize, key_len: usize, before: u64, after: u64) {
    // saturating_sub stops at 0 instead of wrapping: in a release build, plain `after - before`
    // would wrap to an enormous number if memory use ever went down.
    let used_bytes = after.saturating_sub(before) * 1024;
    let per_key = used_bytes as f64 / count as f64;
    let raw = key_len + size_of::<Location>();          // NEW: size_of tells you a type's size in bytes
    println!(
        "{name}: {count} keys of {key_len} bytes used {:.1} MB = {per_key:.0} bytes per key \
         (raw key + location = {raw} bytes, so {:.1}x overhead)",
        used_bytes as f64 / 1_048_576.0,
        per_key / raw as f64
    );
}