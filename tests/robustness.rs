//! Truncated and corrupted inputs must never panic.

use mediainfo::MediaInfo;
use std::path::Path;

fn corpus() -> Vec<(String, Vec<u8>)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut v: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.file_name() != "generate.sh").map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap())).collect();
    v.sort();
    v
}

/// Tiny deterministic PRNG so the test is reproducible.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn truncated_files_do_not_panic() {
    for (name, data) in corpus() {
        for cut in [0usize, 1, 3, 4, 7, 8, 12, 16, 31, 64, 100, 256, 1000, 4096] {
            if cut >= data.len() {
                continue;
            }
            let mut mi = MediaInfo::new();
            mi.open_bytes(data[..cut].to_vec(), Some(&name));
            let _ = mi.inform();
        }
        let half = data.len() / 2;
        let mut mi = MediaInfo::new();
        mi.open_bytes(data[..half].to_vec(), Some(&name));
        let _ = mi.inform();
    }
}

#[test]
fn corrupted_files_do_not_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for (name, data) in corpus() {
        if data.is_empty() {
            continue;
        }
        for round in 0..6 {
            let mut d = data.clone();
            let flips = 1 + round * 4;
            for _ in 0..flips {
                let i = (rng.next() as usize) % d.len();
                d[i] = rng.next() as u8;
            }
            let mut mi = MediaInfo::new();
            mi.open_bytes(d, Some(&name));
            let _ = mi.inform();
            mi.option("Output", "XML");
            let _ = mi.inform();
        }
    }
}
