//! Minimal zero-copy bencode support.
//!
//! Decoding validates the structure of an element once and then hands out
//! [`Value`]s that borrow directly from the input buffer: no allocation is
//! performed while walking dictionaries or lists. Encoding is limited to the
//! two primitives the protocol needs (byte strings and integers); callers are
//! responsible for emitting dictionary keys in sorted order.

use std::fmt;

/// Maximum nesting level accepted while decoding (protects the stack from
/// malicious input).
const MAX_DEPTH: u32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error(&'static str);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// A single, already validated bencode element.
#[derive(Clone, Copy)]
pub struct Value<'a> {
    raw: &'a [u8],
}

/// Decodes the first bencode element of `buf`. Trailing bytes are ignored.
pub fn decode(buf: &[u8]) -> Result<Value<'_>, Error> {
    let end = skip(buf, 0, 0)?;
    Ok(Value { raw: &buf[..end] })
}

/// Returns the position right after the element starting at `pos`,
/// validating it on the way.
fn skip(buf: &[u8], pos: usize, depth: u32) -> Result<usize, Error> {
    if depth > MAX_DEPTH {
        return Err(Error("nesting_too_deep"));
    }
    match buf.get(pos) {
        Some(b'i') => {
            let end = find(buf, pos + 1, b'e').ok_or(Error("unterminated_int"))?;
            parse_int(&buf[pos + 1..end])?;
            Ok(end + 1)
        }
        Some(b'l') => {
            let mut p = pos + 1;
            while *buf.get(p).ok_or(Error("unterminated_list"))? != b'e' {
                p = skip(buf, p, depth + 1)?;
            }
            Ok(p + 1)
        }
        Some(b'd') => {
            let mut p = pos + 1;
            loop {
                match buf.get(p) {
                    Some(b'e') => return Ok(p + 1),
                    Some(b'0'..=b'9') => {
                        p = string_bounds(buf, p)?.1;
                        p = skip(buf, p, depth + 1)?;
                    }
                    Some(_) => return Err(Error("invalid_dict_key")),
                    None => return Err(Error("unterminated_dict")),
                }
            }
        }
        Some(b'0'..=b'9') => Ok(string_bounds(buf, pos)?.1),
        Some(_) => Err(Error("invalid_element")),
        None => Err(Error("unexpected_eof")),
    }
}

/// Returns `(data_start, data_end)` of the byte string starting at `pos`.
fn string_bounds(buf: &[u8], pos: usize) -> Result<(usize, usize), Error> {
    let colon = find(buf, pos, b':').ok_or(Error("unterminated_string_len"))?;
    let digits = &buf[pos..colon];
    if digits.is_empty() || digits.len() > 10 || !digits.iter().all(u8::is_ascii_digit) {
        return Err(Error("invalid_string_len"));
    }
    let len = digits
        .iter()
        .fold(0usize, |acc, d| acc * 10 + (d - b'0') as usize);
    let start = colon + 1;
    let end = start
        .checked_add(len)
        .filter(|&e| e <= buf.len())
        .ok_or(Error("string_out_of_bounds"))?;
    Ok((start, end))
}

fn parse_int(digits: &[u8]) -> Result<i64, Error> {
    std::str::from_utf8(digits)
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or(Error("invalid_int"))
}

fn find(buf: &[u8], from: usize, needle: u8) -> Option<usize> {
    buf.get(from..)?
        .iter()
        .position(|&b| b == needle)
        .map(|i| from + i)
}

impl<'a> Value<'a> {
    pub fn as_int(&self) -> Option<i64> {
        match self.raw.first() {
            Some(b'i') => parse_int(&self.raw[1..self.raw.len() - 1]).ok(),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&'a [u8]> {
        match self.raw.first() {
            Some(b'0'..=b'9') => {
                let (start, end) = string_bounds(self.raw, 0).ok()?;
                Some(&self.raw[start..end])
            }
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&'a str> {
        std::str::from_utf8(self.as_bytes()?).ok()
    }

    /// Iterates over the elements of a list.
    pub fn list(&self) -> Option<ListIter<'a>> {
        (self.raw.first() == Some(&b'l')).then_some(ListIter {
            raw: self.raw,
            pos: 1,
        })
    }

    /// Iterates over the `(key, value)` pairs of a dictionary.
    pub fn dict(&self) -> Option<DictIter<'a>> {
        (self.raw.first() == Some(&b'd')).then_some(DictIter {
            raw: self.raw,
            pos: 1,
        })
    }

    /// Looks up `key` in a dictionary (linear scan, no allocation).
    pub fn get(&self, key: &[u8]) -> Option<Value<'a>> {
        self.dict()?.find(|(k, _)| *k == key).map(|(_, v)| v)
    }
}

impl fmt::Debug for Value<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Value({})", String::from_utf8_lossy(self.raw))
    }
}

pub struct ListIter<'a> {
    raw: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for ListIter<'a> {
    type Item = Value<'a>;

    fn next(&mut self) -> Option<Value<'a>> {
        if *self.raw.get(self.pos)? == b'e' {
            return None;
        }
        // The element was validated by `decode`, so this cannot fail.
        let end = skip(self.raw, self.pos, 0).ok()?;
        let value = Value {
            raw: &self.raw[self.pos..end],
        };
        self.pos = end;
        Some(value)
    }
}

pub struct DictIter<'a> {
    raw: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for DictIter<'a> {
    type Item = (&'a [u8], Value<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        if *self.raw.get(self.pos)? == b'e' {
            return None;
        }
        let (key_start, key_end) = string_bounds(self.raw, self.pos).ok()?;
        let end = skip(self.raw, key_end, 0).ok()?;
        let item = (
            &self.raw[key_start..key_end],
            Value {
                raw: &self.raw[key_end..end],
            },
        );
        self.pos = end;
        Some(item)
    }
}

/// Appends a bencoded byte string (`<len>:<bytes>`).
pub fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    put_decimal(out, bytes.len() as i64);
    out.push(b':');
    out.extend_from_slice(bytes);
}

/// Appends a bencoded integer (`i<value>e`).
pub fn put_int(out: &mut Vec<u8>, value: i64) {
    out.push(b'i');
    put_decimal(out, value);
    out.push(b'e');
}

/// Appends a `key -> integer` dictionary entry.
pub fn put_key_int(out: &mut Vec<u8>, key: &str, value: i64) {
    put_bytes(out, key.as_bytes());
    put_int(out, value);
}

/// Appends the decimal representation of `value` without allocating.
pub fn put_decimal(out: &mut Vec<u8>, value: i64) {
    let mut digits = [0u8; 20];
    let mut n = value.unsigned_abs();
    let mut i = digits.len();
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    if value < 0 {
        out.push(b'-');
    }
    out.extend_from_slice(&digits[i..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_nested_structures() {
        let v = decode(b"d1:ai-12e1:bl3:foo3:bare1:cd1:xi0eee").unwrap();
        assert_eq!(v.get(b"a").unwrap().as_int(), Some(-12));
        let list: Vec<_> = v
            .get(b"b")
            .unwrap()
            .list()
            .unwrap()
            .map(|x| x.as_str().unwrap())
            .collect();
        assert_eq!(list, ["foo", "bar"]);
        assert_eq!(v.get(b"c").unwrap().get(b"x").unwrap().as_int(), Some(0));
        assert!(v.get(b"zz").is_none());
    }

    #[test]
    fn ignores_trailing_bytes() {
        let v = decode(b"4:spamGARBAGE").unwrap();
        assert_eq!(v.as_bytes(), Some(&b"spam"[..]));
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            &b""[..],
            b"i12",
            b"ie",
            b"i1x2e",
            b"5:abc",
            b"l",
            b"d1:a",
            b"di1ei2ee",
            b"x",
            b"99999999999:a",
        ] {
            assert!(
                decode(bad).is_err(),
                "accepted {:?}",
                String::from_utf8_lossy(bad)
            );
        }
        let deep = [vec![b'l'; 100], vec![b'e'; 100]].concat();
        assert!(decode(&deep).is_err());
    }

    #[test]
    fn encodes_primitives() {
        let mut out = Vec::new();
        put_bytes(&mut out, b"spam");
        put_int(&mut out, -1);
        put_int(&mut out, 0);
        put_int(&mut out, i64::MIN);
        assert_eq!(out, b"4:spami-1ei0ei-9223372036854775808e");
    }
}
