//! Small self-contained helpers that replace former third-party crates.

pub mod dns;
pub mod http;

use std::hash::{BuildHasher, Hash, Hasher};

pub fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    s
}

pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    fn nibble(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let s = s.as_bytes();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    s.as_chunks::<2>()
        .0
        .iter()
        .map(|p| Some((nibble(p[0])? << 4) | nibble(p[1])?))
        .collect()
}

/// Standard (RFC 4648) base64 decoder. ASCII whitespace is ignored and the
/// trailing `=` padding is optional.
pub fn base64_decode(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3 + 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut padding = 0;
    for &c in input {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding += 1;
                continue;
            }
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        if padding > 0 {
            return None; // data after padding
        }
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    // A lone 6-bit group cannot encode a byte.
    if padding > 2 || bits >= 6 {
        return None;
    }
    Some(out)
}

/// Small non-cryptographic PRNG (SplitMix64). It is only used for DHT node
/// ids, peer ids, DNS query ids and peer selection.
pub struct Rng(u64);

impl Rng {
    /// Seeds from the OS-backed keys of `RandomState`, the current time and
    /// the thread id, so every instance produces a different sequence.
    pub fn new() -> Self {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        h.write_u128(now);
        std::thread::current().id().hash(&mut h);
        Rng(h.finish())
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform-ish integer in `0..n` (`n` must be > 0).
    pub fn below(&mut self, n: usize) -> usize {
        ((self.next_u64() as u128 * n as u128) >> 64) as usize
    }

    pub fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let r = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&r[..chunk.len()]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        assert_eq!(hex_encode(&[0x00, 0xab, 0xff]), "00abff");
        assert_eq!(hex_decode("00ABff"), Some(vec![0x00, 0xab, 0xff]));
        assert_eq!(hex_decode("abc"), None);
        assert_eq!(hex_decode("zz"), None);
    }

    #[test]
    fn base64_decoding() {
        assert_eq!(base64_decode(b"").unwrap(), b"");
        assert_eq!(base64_decode(b"Zg==").unwrap(), b"f");
        assert_eq!(base64_decode(b"Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode(b"Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode(b"Zm9vYg").unwrap(), b"foob");
        assert_eq!(base64_decode(b"Zm9v\nYmFy").unwrap(), b"foobar");
        assert_eq!(
            base64_decode(b"sJmdb6z7rilEuce7pELu6pzZ1LI=")
                .map(|v| hex_encode(&v))
                .unwrap(),
            "b0999d6facfbae2944b9c7bba442eeea9cd9d4b2"
        );
        assert!(base64_decode(b"Zm9v!").is_none());
        assert!(base64_decode(b"Z").is_none());
        assert!(base64_decode(b"Zg==Zg").is_none());
    }

    #[test]
    fn rng_bounds() {
        let mut rng = Rng::new();
        for n in 1..50 {
            assert!(rng.below(n) < n);
        }
        let mut a = [0u8; 20];
        rng.fill(&mut a);
        assert!(a.iter().any(|&b| b != 0));
    }
}
