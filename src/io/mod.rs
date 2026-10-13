//! Bounded, buffered random-access input. Every read is checked; short reads never panic.

pub mod bits;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const WINDOW: usize = 256 * 1024;

enum Source {
    File(File),
    Memory(Vec<u8>),
}

/// Random-access reader with a cursor and a sliding cache window.
pub struct Reader {
    src: Source,
    len: u64,
    pos: u64,
    win_pos: u64,
    win: Vec<u8>,
    error: Option<std::io::Error>,
}

impl Reader {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let f = File::open(path)?;
        let len = f.metadata()?.len();
        Ok(Self { src: Source::File(f), len, pos: 0, win_pos: 0, win: Vec::new(), error: None })
    }

    pub fn from_bytes(data: Vec<u8>) -> Self {
        let len = data.len() as u64;
        Self { src: Source::Memory(data), len, pos: 0, win_pos: 0, win: Vec::new(), error: None }
    }

    pub fn len(&self) -> u64 {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn pos(&self) -> u64 {
        self.pos
    }
    pub fn remaining(&self) -> u64 {
        self.len.saturating_sub(self.pos)
    }
    pub fn at_end(&self) -> bool {
        self.pos >= self.len
    }

    /// Take the first file I/O error encountered by a read or seek. A file ending before its
    /// recorded length is reported as `UnexpectedEof`; normal bounded reads at EOF are not errors.
    pub fn take_error(&mut self) -> Option<std::io::Error> {
        self.error.take()
    }
    pub fn seek(&mut self, pos: u64) {
        self.pos = pos.min(self.len);
    }
    pub fn skip(&mut self, n: u64) {
        self.pos = self.pos.saturating_add(n).min(self.len);
    }

    fn fill_window(&mut self, pos: u64, min_len: usize) {
        let want = min_len.max(WINDOW).min(self.len.saturating_sub(pos) as usize);
        match &mut self.src {
            Source::Memory(m) => {
                let start = (pos as usize).min(m.len());
                let end = (start + want).min(m.len());
                self.win = m[start..end].to_vec();
            }
            Source::File(f) => {
                self.win.clear();
                self.win.resize(want, 0);
                let mut got = 0;
                match f.seek(SeekFrom::Start(pos)) {
                    Ok(_) => {
                        while got < want {
                            match f.read(&mut self.win[got..]) {
                                Ok(0) => {
                                    self.error.get_or_insert_with(|| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "media file ended before its recorded length"));
                                    break;
                                }
                                Ok(n) => got += n,
                                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                                Err(error) => {
                                    self.error.get_or_insert(error);
                                    break;
                                }
                            }
                        }
                    }
                    Err(error) => { self.error.get_or_insert(error); }
                }
                self.win.truncate(got);
            }
        }
        self.win_pos = pos;
    }

    /// Bytes at an absolute position; shorter than `n` at end of file.
    pub fn read_at(&mut self, pos: u64, n: usize) -> &[u8] {
        if pos >= self.len || n == 0 {
            return &[];
        }
        let n = n.min((self.len - pos) as usize);
        let in_window = pos >= self.win_pos && pos + n as u64 <= self.win_pos + self.win.len() as u64;
        if !in_window {
            self.fill_window(pos, n);
        }
        let off = (pos - self.win_pos) as usize;
        let end = (off + n).min(self.win.len());
        &self.win[off.min(end)..end]
    }

    /// Owned copy of bytes at an absolute position.
    pub fn read_vec_at(&mut self, pos: u64, n: usize) -> Vec<u8> {
        self.read_at(pos, n).to_vec()
    }

    /// Bytes at the cursor without advancing.
    pub fn peek(&mut self, n: usize) -> &[u8] {
        let p = self.pos;
        self.read_at(p, n)
    }

    /// Bytes at the cursor, advancing by what was actually read.
    pub fn read(&mut self, n: usize) -> Vec<u8> {
        let v = self.peek(n).to_vec();
        self.pos += v.len() as u64;
        v
    }

    /// Exactly `n` bytes or `None` (cursor untouched on failure).
    pub fn read_exact(&mut self, n: usize) -> Option<Vec<u8>> {
        if self.remaining() < n as u64 {
            return None;
        }
        let bytes = self.peek(n);
        if bytes.len() != n {
            return None;
        }
        let bytes = bytes.to_vec();
        self.pos += n as u64;
        Some(bytes)
    }

    pub fn read_u8(&mut self) -> Option<u8> {
        let v = *self.peek(1).first()?;
        self.pos += 1;
        Some(v)
    }
    pub fn read_u16be(&mut self) -> Option<u16> {
        self.read_exact(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    pub fn read_u16le(&mut self) -> Option<u16> {
        self.read_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }
    pub fn read_u24be(&mut self) -> Option<u32> {
        self.read_exact(3).map(|b| u32::from_be_bytes([0, b[0], b[1], b[2]]))
    }
    pub fn read_u24le(&mut self) -> Option<u32> {
        self.read_exact(3).map(|b| u32::from_le_bytes([b[0], b[1], b[2], 0]))
    }
    pub fn read_u32be(&mut self) -> Option<u32> {
        self.read_exact(4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn read_u32le(&mut self) -> Option<u32> {
        self.read_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn read_u64be(&mut self) -> Option<u64> {
        self.read_exact(8).map(|b| u64::from_be_bytes(b.try_into().unwrap()))
    }
    pub fn read_u64le(&mut self) -> Option<u64> {
        self.read_exact(8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    }
    pub fn read_f32be(&mut self) -> Option<f32> {
        self.read_u32be().map(f32::from_bits)
    }
    pub fn read_f64be(&mut self) -> Option<f64> {
        self.read_u64be().map(f64::from_bits)
    }
    pub fn read_fourcc(&mut self) -> Option<[u8; 4]> {
        self.read_exact(4).map(|b| [b[0], b[1], b[2], b[3]])
    }
}

// ---- slice helpers shared by parsers

pub fn be16(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_be_bytes([s[0], s[1]]))
}
pub fn le16(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}
pub fn be24(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 3).map(|s| u32::from_be_bytes([0, s[0], s[1], s[2]]))
}
pub fn le24(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 3).map(|s| u32::from_le_bytes([s[0], s[1], s[2], 0]))
}
pub fn be32(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}
pub fn le32(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
pub fn be64(b: &[u8], o: usize) -> Option<u64> {
    b.get(o..o + 8).map(|s| u64::from_be_bytes(s.try_into().unwrap()))
}
pub fn le64(b: &[u8], o: usize) -> Option<u64> {
    b.get(o..o + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
}

/// Lossy UTF-8 decode of a byte slice up to the first NUL.
pub fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// Latin-1 decode (every byte is a code point).
pub fn latin1(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    b[..end].iter().map(|&c| c as char).collect()
}

/// UTF-16 decode with an optional BOM; `be` is the default byte order.
pub fn utf16(b: &[u8], be: bool) -> String {
    let (b, be) = match b {
        [0xFF, 0xFE, rest @ ..] => (rest, false),
        [0xFE, 0xFF, rest @ ..] => (rest, true),
        _ => (b, be),
    };
    let units: Vec<u16> = b.chunks_exact(2).map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&units)
}

/// Remove characters that have no place in a metadata value (control chars, trailing whitespace).
pub fn clean_text(s: &str) -> String {
    s.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect::<String>().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_are_bounded() {
        let mut r = Reader::from_bytes((0..=255u8).collect());
        assert_eq!(r.len(), 256);
        assert_eq!(r.read_u8(), Some(0));
        assert_eq!(r.read_u16be(), Some(0x0102));
        assert_eq!(r.read_u32le(), Some(0x06050403));
        r.seek(254);
        assert_eq!(r.read_u32be(), None);
        assert_eq!(r.pos(), 254);
        assert_eq!(r.read(10), vec![254, 255]);
        assert!(r.at_end());
        assert_eq!(r.read_at(1000, 4), &[]);
        assert_eq!(r.read_at(250, 100), &[250, 251, 252, 253, 254, 255]);
        assert!(r.take_error().is_none());
    }

    #[test]
    fn exact_reads_reject_short_source_without_advancing() {
        struct TempFile(std::path::PathBuf);
        impl Drop for TempFile {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let temp = TempFile(std::env::temp_dir().join(format!("mediainfo-short-read-{}-{nonce}", std::process::id())));
        let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(&temp.0).unwrap();
        file.write_all(&[0; 8]).unwrap();
        let mut r = Reader::open(&temp.0).unwrap();
        // Shrink the file after the reader has recorded its length, before filling its cache.
        file.set_len(1).unwrap();
        assert_eq!(r.len(), 8);
        assert_eq!(r.read_exact(8), None);
        assert_eq!(r.pos(), 0);
        assert_eq!(r.read_u16be(), None);
        assert_eq!(r.read_u32be(), None);
        assert_eq!(r.read_u64be(), None);
        assert_eq!(r.read_fourcc(), None);
        assert_eq!(r.pos(), 0);
        assert_eq!(r.take_error().unwrap().kind(), std::io::ErrorKind::UnexpectedEof);
        assert!(r.take_error().is_none());
        assert_eq!(r.read_u8(), Some(0));
    }

    #[test]
    fn text_decoding() {
        assert_eq!(cstr(b"abc\0def"), "abc");
        assert_eq!(utf16(&[0xFF, 0xFE, b'h', 0, b'i', 0], true), "hi");
        assert_eq!(utf16(&[0, b'h', 0, b'i'], true), "hi");
        assert_eq!(latin1(&[0xE9]), "é");
    }
}
