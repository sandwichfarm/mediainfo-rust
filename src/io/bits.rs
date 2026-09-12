//! MSB-first bit reader with Exp-Golomb support (H.26x / MPEG headers).

pub struct BitReader<'a> {
    data: &'a [u8],
    pos: usize, // in bits
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn bits_left(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.pos)
    }
    pub fn bit_pos(&self) -> usize {
        self.pos
    }
    pub fn byte_pos(&self) -> usize {
        self.pos.div_ceil(8)
    }
    pub fn is_byte_aligned(&self) -> bool {
        self.pos % 8 == 0
    }
    pub fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
    pub fn skip(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n).min(self.data.len() * 8);
    }
    pub fn seek_bits(&mut self, pos: usize) {
        self.pos = pos.min(self.data.len() * 8);
    }

    pub fn bit(&mut self) -> Option<bool> {
        self.bits(1).map(|v| v == 1)
    }

    /// Up to 64 bits, MSB first.
    pub fn bits(&mut self, n: usize) -> Option<u64> {
        if n == 0 {
            return Some(0);
        }
        if n > 64 || self.bits_left() < n {
            return None;
        }
        let mut v: u64 = 0;
        let mut left = n;
        while left > 0 {
            let byte = self.data[self.pos / 8];
            let bit_off = self.pos % 8;
            let avail = 8 - bit_off;
            let take = avail.min(left);
            let chunk = ((byte as u64) >> (avail - take)) & ((1u64 << take) - 1);
            v = (v << take) | chunk;
            self.pos += take;
            left -= take;
        }
        Some(v)
    }

    pub fn u8(&mut self, n: usize) -> Option<u8> {
        self.bits(n).map(|v| v as u8)
    }
    pub fn u16(&mut self, n: usize) -> Option<u16> {
        self.bits(n).map(|v| v as u16)
    }
    pub fn u32(&mut self, n: usize) -> Option<u32> {
        self.bits(n).map(|v| v as u32)
    }

    /// Peek without advancing.
    pub fn peek(&self, n: usize) -> Option<u64> {
        let mut c = BitReader { data: self.data, pos: self.pos };
        c.bits(n)
    }

    /// Unsigned Exp-Golomb (ue(v)).
    pub fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == false {
            zeros += 1;
            if zeros > 32 {
                return None;
            }
        }
        if zeros == 0 {
            return Some(0);
        }
        let rest = self.bits(zeros)?;
        Some(((1u64 << zeros) - 1 + rest) as u32)
    }

    /// Signed Exp-Golomb (se(v)).
    pub fn se(&mut self) -> Option<i32> {
        let k = self.ue()? as i64;
        Some(if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) } as i32)
    }

    pub fn bytes(&mut self, n: usize) -> Option<Vec<u8>> {
        (0..n).map(|_| self.u8(8)).collect()
    }
}

/// Remove H.264/HEVC emulation-prevention bytes (`00 00 03` → `00 00`).
pub fn unescape_rbsp(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut zeros = 0;
    for &b in data {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        out.push(b);
        if b == 0 {
            zeros += 1;
        } else {
            zeros = 0;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_and_golomb() {
        let d = [0b1010_0000u8, 0b0110_0000];
        let mut r = BitReader::new(&d);
        assert_eq!(r.bits(3), Some(0b101));
        assert_eq!(r.bits(6), Some(0b000000));
        assert_eq!(r.bits_left(), 7);
        // ue: 1 -> 0 ; 010 -> 1 ; 011 -> 2 ; 00100 -> 3
        let d = [0b1010_0110u8, 0b0100_0000];
        let mut r = BitReader::new(&d);
        assert_eq!(r.ue(), Some(0));
        assert_eq!(r.ue(), Some(1));
        assert_eq!(r.ue(), Some(2));
        assert_eq!(r.ue(), Some(3));
        let d = [0b0100_1100u8];
        let mut r = BitReader::new(&d);
        assert_eq!(r.se(), Some(1)); // 010 -> k=1 -> +1
        assert_eq!(r.se(), Some(-1)); // 011 -> k=2 -> -1
        assert_eq!(BitReader::new(&[0xFF]).bits(65), None);
        assert_eq!(BitReader::new(&[]).bits(1), None);
    }

    #[test]
    fn rbsp() {
        assert_eq!(unescape_rbsp(&[0, 0, 3, 1, 0, 0, 3, 0]), vec![0, 0, 1, 0, 0, 0]);
    }
}
