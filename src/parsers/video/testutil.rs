//! Test-only helpers shared by the video codec unit tests: an MSB-first bit writer for hand-built
//! headers.

/// MSB-first bit writer (mirror of `io::bits::BitReader`).
pub struct BitWriter {
    bytes: Vec<u8>,
    bits: usize,
}

impl Default for BitWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl BitWriter {
    pub fn new() -> Self {
        Self { bytes: Vec::new(), bits: 0 }
    }

    /// Append the low `n` bits of `v`, MSB first.
    pub fn b(&mut self, v: u64, n: usize) {
        for i in (0..n).rev() {
            if self.bits % 8 == 0 {
                self.bytes.push(0);
            }
            if (v >> i) & 1 == 1 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> (self.bits % 8);
            }
            self.bits += 1;
        }
    }

    /// Unsigned Exp-Golomb.
    pub fn ue(&mut self, v: u32) {
        let x = v as u64 + 1;
        let len = 64 - x.leading_zeros() as usize;
        self.b(0, len - 1);
        self.b(x, len);
    }

    /// Signed Exp-Golomb.
    pub fn se(&mut self, v: i32) {
        let k = if v > 0 { 2 * v as u32 - 1 } else { (-2 * v) as u32 };
        self.ue(k);
    }

    /// Append whole bytes (aligns first).
    pub fn bytes(&mut self, d: &[u8]) {
        self.align();
        self.bytes.extend_from_slice(d);
        self.bits = self.bytes.len() * 8;
    }

    pub fn align(&mut self) {
        while self.bits % 8 != 0 {
            self.b(0, 1);
        }
    }

    /// Finish with RBSP trailing bits (a 1 then zero padding).
    pub fn trailing(mut self) -> Vec<u8> {
        self.b(1, 1);
        self.align();
        self.bytes
    }

    /// Finish with zero padding.
    pub fn done(mut self) -> Vec<u8> {
        self.align();
        self.bytes
    }
}
