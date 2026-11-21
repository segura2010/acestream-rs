//! Minimal AES-128 (FIPS-197) block cipher plus the CBC decryption used for
//! AceStream transport files.
//!
//! The data processed per run is a single torrent (well under 1 KiB), so a
//! compact byte-oriented implementation is used instead of large T-tables.
//! The S-boxes are generated at compile time.

const fn xtime(x: u8) -> u8 {
    (x << 1) ^ if x & 0x80 != 0 { 0x1b } else { 0 }
}

/// Multiplication in GF(2^8).
const fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    p
}

/// Builds the S-box and its inverse by walking the multiplicative group with
/// the generator 3 (p) and its inverse (q).
const fn build_sboxes() -> ([u8; 256], [u8; 256]) {
    let mut sbox = [0u8; 256];
    let mut inv = [0u8; 256];
    let mut p: u8 = 1;
    let mut q: u8 = 1;
    loop {
        p ^= xtime(p); // p * 3
        q ^= q << 1; // q / 3
        q ^= q << 2;
        q ^= q << 4;
        if q & 0x80 != 0 {
            q ^= 0x09;
        }
        let s =
            q ^ q.rotate_left(1) ^ q.rotate_left(2) ^ q.rotate_left(3) ^ q.rotate_left(4) ^ 0x63;
        sbox[p as usize] = s;
        inv[s as usize] = p;
        if p == 1 {
            break;
        }
    }
    sbox[0] = 0x63;
    inv[0x63] = 0;
    (sbox, inv)
}

const SBOXES: ([u8; 256], [u8; 256]) = build_sboxes();
const SBOX: [u8; 256] = SBOXES.0;
const INV_SBOX: [u8; 256] = SBOXES.1;

pub struct Aes128 {
    round_keys: [[u8; 16]; 11],
}

impl Aes128 {
    pub fn new(key: &[u8; 16]) -> Self {
        let mut words = [[0u8; 4]; 44];
        for (i, w) in words.iter_mut().take(4).enumerate() {
            w.copy_from_slice(&key[4 * i..4 * i + 4]);
        }
        let mut rcon = 1u8;
        for i in 4..44 {
            let mut t = words[i - 1];
            if i % 4 == 0 {
                t = [
                    SBOX[t[1] as usize] ^ rcon,
                    SBOX[t[2] as usize],
                    SBOX[t[3] as usize],
                    SBOX[t[0] as usize],
                ];
                rcon = xtime(rcon);
            }
            for j in 0..4 {
                words[i][j] = words[i - 4][j] ^ t[j];
            }
        }
        let mut round_keys = [[0u8; 16]; 11];
        for (r, rk) in round_keys.iter_mut().enumerate() {
            for c in 0..4 {
                rk[4 * c..4 * c + 4].copy_from_slice(&words[4 * r + c]);
            }
        }
        Aes128 { round_keys }
    }

    pub fn decrypt_block(&self, s: &mut [u8; 16]) {
        xor(s, &self.round_keys[10]);
        for round in (1..10).rev() {
            inv_shift_rows(s);
            s.iter_mut().for_each(|b| *b = INV_SBOX[*b as usize]);
            xor(s, &self.round_keys[round]);
            inv_mix_columns(s);
        }
        inv_shift_rows(s);
        s.iter_mut().for_each(|b| *b = INV_SBOX[*b as usize]);
        xor(s, &self.round_keys[0]);
    }

    #[cfg(test)]
    pub fn encrypt_block(&self, s: &mut [u8; 16]) {
        xor(s, &self.round_keys[0]);
        for (round, round_key) in self.round_keys.iter().enumerate().skip(1) {
            s.iter_mut().for_each(|b| *b = SBOX[*b as usize]);
            shift_rows(s);
            if round != 10 {
                mix_columns(s);
            }
            xor(s, round_key);
        }
    }
}

fn xor(s: &mut [u8; 16], other: &[u8; 16]) {
    s.iter_mut().zip(other).for_each(|(a, b)| *a ^= b);
}

// The state is stored column-major: byte `row + 4 * col`.
fn inv_shift_rows(s: &mut [u8; 16]) {
    let old = *s;
    for r in 1..4 {
        for c in 0..4 {
            s[r + 4 * c] = old[r + 4 * ((c + 4 - r) % 4)];
        }
    }
}

fn inv_mix_columns(s: &mut [u8; 16]) {
    for col in s.as_chunks_mut::<4>().0 {
        let [a0, a1, a2, a3] = [col[0], col[1], col[2], col[3]];
        col[0] = gmul(a0, 14) ^ gmul(a1, 11) ^ gmul(a2, 13) ^ gmul(a3, 9);
        col[1] = gmul(a0, 9) ^ gmul(a1, 14) ^ gmul(a2, 11) ^ gmul(a3, 13);
        col[2] = gmul(a0, 13) ^ gmul(a1, 9) ^ gmul(a2, 14) ^ gmul(a3, 11);
        col[3] = gmul(a0, 11) ^ gmul(a1, 13) ^ gmul(a2, 9) ^ gmul(a3, 14);
    }
}

#[cfg(test)]
fn shift_rows(s: &mut [u8; 16]) {
    let old = *s;
    for r in 1..4 {
        for c in 0..4 {
            s[r + 4 * c] = old[r + 4 * ((c + r) % 4)];
        }
    }
}

#[cfg(test)]
fn mix_columns(s: &mut [u8; 16]) {
    for col in s.as_chunks_mut::<4>().0 {
        let [a0, a1, a2, a3] = [col[0], col[1], col[2], col[3]];
        col[0] = gmul(a0, 2) ^ gmul(a1, 3) ^ a2 ^ a3;
        col[1] = a0 ^ gmul(a1, 2) ^ gmul(a2, 3) ^ a3;
        col[2] = a0 ^ a1 ^ gmul(a2, 2) ^ gmul(a3, 3);
        col[3] = gmul(a0, 3) ^ a1 ^ a2 ^ gmul(a3, 2);
    }
}

/// AES-128-CBC decryption.
///
/// Mirrors the behaviour of the original implementation: a trailing partial
/// block is padded before decryption, and PKCS#7-style padding is removed
/// only when every padding byte is consistent (otherwise the data is
/// returned untouched).
pub fn decrypt_cbc(key: &[u8; 16], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let cipher = Aes128::new(key);
    let mut out = Vec::with_capacity(data.len().div_ceil(16) * 16);
    let mut prev = *iv;
    for chunk in data.chunks(16) {
        let mut block = [(16 - chunk.len()) as u8; 16];
        block[..chunk.len()].copy_from_slice(chunk);
        let ciphertext = block;
        cipher.decrypt_block(&mut block);
        xor(&mut block, &prev);
        prev = ciphertext;
        out.extend_from_slice(&block);
    }

    if let Some(&pad) = out.last() {
        let pad_len = pad as usize;
        if pad_len < out.len() {
            let start = out.len() - pad_len;
            if out[start..].iter().all(|&b| b == pad) {
                out.truncate(start);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::{hex_decode, hex_encode};

    fn arr16(hex: &str) -> [u8; 16] {
        hex_decode(hex).unwrap().try_into().unwrap()
    }

    #[test]
    fn sbox_values() {
        assert_eq!(SBOX[0x00], 0x63);
        assert_eq!(SBOX[0x01], 0x7c);
        assert_eq!(SBOX[0x53], 0xed);
        assert_eq!(SBOX[0xff], 0x16);
        for i in 0..256 {
            assert_eq!(INV_SBOX[SBOX[i] as usize] as usize, i);
        }
    }

    #[test]
    fn fips197_vector() {
        let cipher = Aes128::new(&arr16("000102030405060708090a0b0c0d0e0f"));
        let mut block = arr16("00112233445566778899aabbccddeeff");
        cipher.encrypt_block(&mut block);
        assert_eq!(hex_encode(&block), "69c4e0d86a7b0430d8cdb78070b4c55a");
        cipher.decrypt_block(&mut block);
        assert_eq!(hex_encode(&block), "00112233445566778899aabbccddeeff");
    }

    #[test]
    fn ecb_block_matches_openssl() {
        let cipher = Aes128::new(&arr16("01020304050607080910111213141516"));
        let mut block: [u8; 16] = *b"Hello world12345";
        cipher.encrypt_block(&mut block);
        assert_eq!(hex_encode(&block), "3effc263dbb3ba30c20b6c8e74524e26");
    }

    #[test]
    fn cbc_without_padding() {
        let key = arr16("01020304050607080910111213141516");
        let data = hex_decode("a0b13fbc896dd69b802d2b49ba2f7cd4").unwrap();
        assert_eq!(decrypt_cbc(&key, &key, &data), b"Hello world12345");
    }

    #[test]
    fn cbc_with_pkcs7_padding() {
        let data =
            hex_decode("a6f397f135af6dac32f5a9cd13c017407469f70d870796577fca2e43a02eaee0").unwrap();
        let out = decrypt_cbc(
            &crate::acestream::crypto::AES_KEY,
            &crate::acestream::crypto::AES_IV,
            &data,
        );
        assert_eq!(out, b"acestream payload!");
    }
}
