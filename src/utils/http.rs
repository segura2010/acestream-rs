//! Tiny blocking HTTP/1.1 client: just enough for the AceStream API (plain
//! HTTP `GET`, chunked or length-delimited responses, simple redirects).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_SIZE: u64 = 16 << 20;
const MAX_REDIRECTS: usize = 10;

/// Performs a `GET` request and returns the response body.
pub fn get(url: &str, user_agent: &str) -> Result<Vec<u8>, String> {
    let mut url = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        match request(&url, user_agent)? {
            Reply::Body(body) => return Ok(body),
            Reply::Redirect(location) => {
                url = if location.starts_with('/') {
                    let (host, _, _) = split_url(&url)?;
                    format!("http://{host}{location}")
                } else {
                    location
                };
            }
        }
    }
    Err("too_many_redirects".into())
}

enum Reply {
    Body(Vec<u8>),
    Redirect(String),
}

/// Splits `http://host[:port]/path` into `(authority, port, path)`.
fn split_url(url: &str) -> Result<(&str, u16, &str), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("unsupported_url:{url}"))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let port = match authority.rsplit_once(':') {
        Some((_, p)) => p.parse().map_err(|_| format!("invalid_port:{url}"))?,
        None => 80,
    };
    // The fragment is never sent to the server.
    let path = path.split('#').next().unwrap_or("/");
    Ok((authority, port, path))
}

fn request(url: &str, user_agent: &str) -> Result<Reply, String> {
    let (authority, port, path) = split_url(url)?;
    let host = authority.rsplit_once(':').map_or(authority, |(h, _)| h);

    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("dns_error:{e}"))?;
    let mut last_err = String::from("no_address");
    let mut stream = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, TIMEOUT) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    let mut stream = stream.ok_or(last_err)?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(TIMEOUT))
        .map_err(|e| e.to_string())?;

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {authority}\r\nUser-Agent: {user_agent}\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        percent_encode(path)
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| e.to_string())?;

    let mut raw = Vec::with_capacity(4096);
    stream
        .take(MAX_RESPONSE_SIZE)
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    parse_response(raw)
}

fn parse_response(raw: Vec<u8>) -> Result<Reply, String> {
    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("incomplete_http_headers")?;
    let head = std::str::from_utf8(&raw[..head_end]).map_err(|_| "invalid_http_headers")?;
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or("invalid_http_status")?;

    let mut chunked = false;
    let mut content_length = None;
    let mut location = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value.to_ascii_lowercase().contains("chunked");
        } else if name.eq_ignore_ascii_case("content-length") {
            content_length = value.parse::<usize>().ok();
        } else if name.eq_ignore_ascii_case("location") {
            location = Some(value.to_owned());
        }
    }

    if (300..400).contains(&status)
        && let Some(location) = location
    {
        return Ok(Reply::Redirect(location));
    }

    let mut body = raw;
    body.drain(..head_end + 4);
    if chunked {
        body = dechunk(&body).ok_or("invalid_chunked_body")?;
    } else if let Some(len) = content_length {
        if body.len() < len {
            return Err("incomplete_http_body".into());
        }
        body.truncate(len);
    }
    Ok(Reply::Body(body))
}

fn dechunk(mut data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    loop {
        let line_end = data.windows(2).position(|w| w == b"\r\n")?;
        let size_field = std::str::from_utf8(&data[..line_end]).ok()?;
        let size_field = size_field.split(';').next()?.trim();
        let size = usize::from_str_radix(size_field, 16).ok()?;
        data = &data[line_end + 2..];
        if size == 0 {
            return Some(out); // trailers (if any) are ignored
        }
        if data.len() < size + 2 {
            return None;
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
}

/// Percent-encodes the characters that are never valid in a request target.
fn percent_encode(target: &str) -> String {
    let mut out = String::with_capacity(target.len());
    for &b in target.as_bytes() {
        if b <= 0x20 || b >= 0x7f || matches!(b, b'"' | b'<' | b'>') {
            out.push_str(&format!("%{b:02X}"));
        } else {
            out.push(b as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_urls() {
        assert_eq!(
            split_url("http://example.org/a?b=c").unwrap(),
            ("example.org", 80, "/a?b=c")
        );
        assert_eq!(
            split_url("http://example.org:8080").unwrap(),
            ("example.org:8080", 8080, "/")
        );
        assert_eq!(split_url("http://h/x#frag").unwrap(), ("h", 80, "/x"));
        assert!(split_url("https://example.org/").is_err());
    }

    #[test]
    fn parses_chunked_response() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n6;ext=1\r\npedia \r\n0\r\n\r\n".to_vec();
        match parse_response(raw).unwrap() {
            Reply::Body(b) => assert_eq!(b, b"Wikipedia "),
            Reply::Redirect(_) => panic!(),
        }
    }

    #[test]
    fn parses_length_delimited_response() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabcdef".to_vec();
        match parse_response(raw).unwrap() {
            Reply::Body(b) => assert_eq!(b, b"abc"),
            Reply::Redirect(_) => panic!(),
        }
    }

    #[test]
    fn parses_redirect() {
        let raw = b"HTTP/1.1 302 Found\r\nLocation: http://other/x\r\n\r\n".to_vec();
        assert!(
            matches!(parse_response(raw).unwrap(), Reply::Redirect(l) if l == "http://other/x")
        );
    }

    #[test]
    fn encodes_target() {
        assert_eq!(percent_encode("/a?b=c d\"<>"), "/a?b=c%20d%22%3C%3E");
    }
}
