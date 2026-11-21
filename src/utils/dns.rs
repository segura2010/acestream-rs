//! Minimal DNS client (IPv4 `A` records over UDP).
//!
//! Used to resolve the DHT bootstrap routers through public resolvers, like
//! the original implementation did (Google DNS, up to 5 attempts alternating
//! servers, 5 s timeout per attempt). All hosts are resolved concurrently
//! from a single socket instead of one after another.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use super::Rng;

const TYPE_A: u16 = 1;
const CLASS_IN: u16 = 1;

/// Resolves every host to its IPv4 addresses. The result has one entry per
/// host, in the same order.
pub fn resolve_ipv4(
    hosts: &[&str],
    servers: &[SocketAddr],
    timeout: Duration,
    attempts: u32,
) -> Vec<Result<Vec<Ipv4Addr>, String>> {
    let mut results: Vec<Option<Result<Vec<Ipv4Addr>, String>>> = hosts
        .iter()
        .map(|h| h.parse::<Ipv4Addr>().ok().map(|ip| Ok(vec![ip])))
        .collect();

    let socket = match UdpSocket::bind(("0.0.0.0", 0)) {
        Ok(s) => s,
        Err(e) => {
            return hosts
                .iter()
                .map(|_| Err(format!("dns_socket:{e}")))
                .collect();
        }
    };

    let base_id = Rng::new().next_u64() as u16;
    let queries: Vec<Option<Vec<u8>>> = hosts
        .iter()
        .enumerate()
        .map(|(i, h)| build_query(base_id.wrapping_add(i as u16), h))
        .collect();
    for (result, query) in results.iter_mut().zip(&queries) {
        if result.is_none() && query.is_none() {
            *result = Some(Err("invalid_hostname".into()));
        }
    }

    let mut buf = [0u8; 1500];
    for attempt in 0..attempts as usize {
        if results.iter().all(Option::is_some) || servers.is_empty() {
            break;
        }
        let server = servers[attempt % servers.len()];
        for (result, query) in results.iter().zip(&queries) {
            if let (None, Some(q)) = (result, query) {
                let _ = socket.send_to(q, server);
            }
        }

        let deadline = Instant::now() + timeout;
        while results.iter().any(Option::is_none) {
            let now = Instant::now();
            if now >= deadline || socket.set_read_timeout(Some(deadline - now)).is_err() {
                break;
            }
            let Ok((n, from)) = socket.recv_from(&mut buf) else {
                break;
            };
            if !servers.contains(&from) || n < 12 {
                continue;
            }
            let idx = u16::from_be_bytes([buf[0], buf[1]]).wrapping_sub(base_id) as usize;
            if let Some(slot @ None) = results.get_mut(idx)
                && let Some(parsed) = parse_response(&buf[..n])
            {
                *slot = Some(parsed);
            }
        }
    }

    results
        .into_iter()
        .map(|r| r.unwrap_or_else(|| Err("dns_timeout".into())))
        .collect()
}

fn build_query(id: u16, host: &str) -> Option<Vec<u8>> {
    let mut q = Vec::with_capacity(18 + host.len());
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00]); // standard query, recursion desired
    q.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]); // 1 question
    for label in host.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&TYPE_A.to_be_bytes());
    q.extend_from_slice(&CLASS_IN.to_be_bytes());
    Some(q)
}

/// Parses a response. Returns `None` for packets that are not a valid answer
/// (they are ignored and the query keeps waiting).
fn parse_response(msg: &[u8]) -> Option<Result<Vec<Ipv4Addr>, String>> {
    let u16_at = |pos: usize| -> Option<u16> {
        Some(u16::from_be_bytes([*msg.get(pos)?, *msg.get(pos + 1)?]))
    };
    let flags = u16_at(2)?;
    if flags & 0x8000 == 0 {
        return None; // not a response
    }
    let rcode = flags & 0x000f;
    if rcode != 0 {
        return Some(Err(format!("dns_rcode:{rcode}")));
    }
    let questions = u16_at(4)?;
    let answers = u16_at(6)?;

    let mut pos = 12;
    for _ in 0..questions {
        pos = skip_name(msg, pos)? + 4;
    }
    let mut ips = Vec::new();
    for _ in 0..answers {
        pos = skip_name(msg, pos)?;
        let rtype = u16_at(pos)?;
        let class = u16_at(pos + 2)?;
        let len = u16_at(pos + 8)? as usize;
        let data = msg.get(pos + 10..pos + 10 + len)?;
        if rtype == TYPE_A && class == CLASS_IN && len == 4 {
            ips.push(Ipv4Addr::new(data[0], data[1], data[2], data[3]));
        }
        pos += 10 + len;
    }
    Some(if ips.is_empty() {
        Err("name_not_found".into())
    } else {
        Ok(ips)
    })
}

fn skip_name(msg: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let len = *msg.get(pos)? as usize;
        match len {
            0 => return Some(pos + 1),
            l if l & 0xc0 == 0xc0 => return Some(pos + 2), // compression pointer
            l => pos += 1 + l,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_query() {
        let q = build_query(0x1234, "a.bc").unwrap();
        assert_eq!(
            q,
            [
                0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, b'a', 2, b'b', b'c', 0, 0, 1, 0, 1
            ]
        );
        assert!(build_query(1, "a..b").is_none());
    }

    #[test]
    fn parses_answers_with_compression() {
        let mut msg = vec![0x12, 0x34, 0x81, 0x80, 0, 1, 0, 2, 0, 0, 0, 0];
        msg.extend_from_slice(&[1, b'a', 2, b'b', b'c', 0, 0, 1, 0, 1]);
        // CNAME answer (ignored) followed by an A record, both using pointers.
        msg.extend_from_slice(&[0xc0, 12, 0, 5, 0, 1, 0, 0, 0, 60, 0, 2, 0xc0, 12]);
        msg.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 10, 0, 0, 7]);
        assert_eq!(
            parse_response(&msg).unwrap().unwrap(),
            vec![Ipv4Addr::new(10, 0, 0, 7)]
        );
    }

    #[test]
    fn reports_errors() {
        let nxdomain = [0x12, 0x34, 0x81, 0x83, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(parse_response(&nxdomain).unwrap().is_err());
        let query = [0x12, 0x34, 0x01, 0x00, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(parse_response(&query).is_none());
    }

    #[test]
    fn ip_literals_skip_dns() {
        let r = resolve_ipv4(&["1.2.3.4"], &[], Duration::from_millis(1), 1);
        assert_eq!(r[0].as_ref().unwrap(), &vec![Ipv4Addr::new(1, 2, 3, 4)]);
    }
}
