//! Minimal mainline DHT client: it only performs `get_peers` lookups for a
//! single infohash.
//!
//! The `get_peers` query is serialised once; sending it to a node only
//! rewrites the transaction id, so a sweep over thousands of nodes does not
//! allocate. Responses are parsed in place from the receive buffer.

use std::collections::HashSet;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::Duration;

use crate::bencode;
use crate::utils::{Rng, dns};

/// Bootstrap routers / trackers queried on every sweep.
const ROUTERS: [(&str, u16); 10] = [
    ("tracker.opentrackr.org", 1337),
    ("9.rarbg.com", 2810),
    ("tracker.openbittorrent.com", 6969),
    ("router.bittorrent.com", 6881),
    ("router.utorrent.com", 6881),
    ("tracker.coppersurfer.tk", 6969),
    ("dht.transmissionbt.com", 6881),
    ("dht.aelitis.com", 6881),
    ("router.silotis.us", 6881),
    ("dht.libtorrent.org", 25401),
];

/// Router names are resolved through public DNS servers (as the original
/// implementation did) instead of the system resolver.
const DNS_SERVERS: [SocketAddr; 2] = [
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(8, 8, 8, 8), 53)),
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(8, 8, 4, 4), 53)),
];
const DNS_TIMEOUT: Duration = Duration::from_secs(5);
const DNS_ATTEMPTS: u32 = 5;

const RECV_BUF_LEN: usize = 4096;
const NODE_INFO_LEN: usize = 26; // 20 bytes id + 4 bytes IPv4 + 2 bytes port
const PEER_INFO_LEN: usize = 6; // 4 bytes IPv4 + 2 bytes port

pub struct DhtManager {
    node_id: [u8; 20],
    routers: Vec<SocketAddrV4>,
    nodes: HashSet<SocketAddrV4>,
    peers: HashSet<SocketAddrV4>,
    querier: Option<Querier>,
    recv_buf: Vec<u8>,
}

/// UDP socket plus the pre-serialised `get_peers` query.
struct Querier {
    socket: UdpSocket,
    query: Vec<u8>,
    prefix_len: usize,
    transaction_id: i32,
}

impl Querier {
    fn new(socket: UdpSocket, node_id: &[u8; 20], infohash: &[u8; 20]) -> Self {
        let query = get_peers_prefix(node_id, infohash);
        Querier {
            socket,
            prefix_len: query.len(),
            query,
            transaction_id: 0,
        }
    }

    fn send(&mut self, addr: SocketAddrV4) {
        self.query.truncate(self.prefix_len);
        finish_get_peers(&mut self.query, self.transaction_id);
        self.transaction_id = self.transaction_id.wrapping_add(1);
        let _ = self.socket.send_to(&self.query, addr);
    }
}

/// Everything in the bencoded `get_peers` query up to the transaction id.
/// Keys are emitted in sorted order, as required by bencode.
fn get_peers_prefix(node_id: &[u8; 20], infohash: &[u8; 20]) -> Vec<u8> {
    let mut q = Vec::with_capacity(128);
    q.extend_from_slice(b"d1:ad");
    bencode::put_bytes(&mut q, b"id");
    bencode::put_bytes(&mut q, node_id);
    bencode::put_bytes(&mut q, b"info_hash");
    bencode::put_bytes(&mut q, infohash);
    q.extend_from_slice(b"4:wantl2:n4ee"); // IPv4 nodes only
    q.extend_from_slice(b"1:q9:get_peers2:roi1e1:ti");
    q
}

fn finish_get_peers(q: &mut Vec<u8>, transaction_id: i32) {
    bencode::put_decimal(q, transaction_id as i64);
    q.extend_from_slice(b"e1:y1:qe");
}

/// Resolves the bootstrap routers and creates the manager.
pub fn new_default() -> DhtManager {
    let hosts: Vec<&str> = ROUTERS.iter().map(|(host, _)| *host).collect();
    let resolved = dns::resolve_ipv4(&hosts, &DNS_SERVERS, DNS_TIMEOUT, DNS_ATTEMPTS);
    let routers = ROUTERS
        .iter()
        .zip(resolved)
        .filter_map(|((_, port), ips)| {
            Some(
                ips.ok()?
                    .into_iter()
                    .map(move |ip| SocketAddrV4::new(ip, *port)),
            )
        })
        .flatten()
        .collect();

    let mut node_id = [0u8; 20];
    Rng::new().fill(&mut node_id);
    DhtManager {
        node_id,
        routers,
        nodes: HashSet::with_capacity(1024),
        peers: HashSet::with_capacity(256),
        querier: None,
        recv_buf: vec![0; RECV_BUF_LEN],
    }
}

impl DhtManager {
    pub fn listen(&mut self, port: u16, infohash: &[u8; 20]) -> io::Result<()> {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        socket.set_read_timeout(Some(Duration::from_millis(300)))?;
        socket.set_write_timeout(Some(Duration::from_millis(1000)))?;
        self.querier = Some(Querier::new(socket, &self.node_id, infohash));
        Ok(())
    }

    /// Sends a `get_peers` query to every router and every known node.
    pub fn get_peers(&mut self) {
        if let Some(q) = &mut self.querier {
            for &addr in self.routers.iter().chain(&self.nodes) {
                q.send(addr);
            }
        }
    }

    /// Waits (up to the socket timeout) for one response, records the
    /// nodes and peers it contains, and immediately queries new nodes.
    /// Returns `false` when nothing valid was received.
    pub fn recv_peers(&mut self) -> bool {
        let Some(q) = &mut self.querier else {
            return false;
        };
        let Ok((n, _)) = q.socket.recv_from(&mut self.recv_buf) else {
            return false;
        };
        let nodes = &mut self.nodes;
        let peers = &mut self.peers;
        parse_response(
            &self.recv_buf[..n],
            |node| {
                if nodes.insert(node) {
                    q.send(node);
                }
            },
            |peer| {
                peers.insert(peer);
            },
        )
    }

    pub fn peers(&self) -> Vec<SocketAddrV4> {
        self.peers.iter().copied().collect()
    }

    pub fn peers_len(&self) -> usize {
        self.peers.len()
    }

    pub fn nodes_len(&self) -> usize {
        self.nodes.len()
    }
}

/// Walks the compact node and peer lists of a DHT response.
fn parse_response(
    msg: &[u8],
    mut on_node: impl FnMut(SocketAddrV4),
    mut on_peer: impl FnMut(SocketAddrV4),
) -> bool {
    let Some(r) = bencode::decode(msg).ok().and_then(|m| m.get(b"r")) else {
        return false;
    };
    if let Some(nodes) = r.get(b"nodes").and_then(|v| v.as_bytes()) {
        for info in nodes.as_chunks::<NODE_INFO_LEN>().0 {
            let addr = compact_addr(&info[20..]);
            if addr.port() != 0 {
                on_node(addr);
            }
        }
    }
    if let Some(values) = r.get(b"values").and_then(|v| v.list()) {
        for value in values {
            if let Some(info) = value.as_bytes()
                && info.len() == PEER_INFO_LEN
            {
                on_peer(compact_addr(info));
            }
        }
    }
    true
}

fn compact_addr(b: &[u8]) -> SocketAddrV4 {
    SocketAddrV4::new(
        Ipv4Addr::new(b[0], b[1], b[2], b[3]),
        u16::from_be_bytes([b[4], b[5]]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_peers_query_matches_original_encoding() {
        // Bytes produced by the original serde_bencode based implementation
        // for node id 0..20, infohash 100..120 and transaction id 1234.
        let node_id: [u8; 20] = std::array::from_fn(|i| i as u8);
        let infohash: [u8; 20] = std::array::from_fn(|i| 100 + i as u8);
        let mut q = get_peers_prefix(&node_id, &infohash);
        finish_get_peers(&mut q, 1234);
        let expected: &[u8] = &[
            100, 49, 58, 97, 100, 50, 58, 105, 100, 50, 48, 58, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
            11, 12, 13, 14, 15, 16, 17, 18, 19, 57, 58, 105, 110, 102, 111, 95, 104, 97, 115, 104,
            50, 48, 58, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114,
            115, 116, 117, 118, 119, 52, 58, 119, 97, 110, 116, 108, 50, 58, 110, 52, 101, 101, 49,
            58, 113, 57, 58, 103, 101, 116, 95, 112, 101, 101, 114, 115, 50, 58, 114, 111, 105, 49,
            101, 49, 58, 116, 105, 49, 50, 51, 52, 101, 49, 58, 121, 49, 58, 113, 101,
        ];
        assert_eq!(q, expected);

        q.truncate(get_peers_prefix(&node_id, &infohash).len());
        finish_get_peers(&mut q, -5);
        assert!(q.ends_with(b"1:ti-5e1:y1:qe"));
    }

    #[test]
    fn parses_get_peers_response() {
        const INPUT: [u8; 356] = [
            100, 49, 58, 114, 100, 50, 58, 105, 100, 50, 48, 58, 60, 56, 25, 141, 42, 131, 140,
            155, 130, 154, 204, 134, 244, 101, 138, 80, 149, 76, 178, 239, 50, 58, 105, 112, 52,
            58, 87, 219, 33, 178, 53, 58, 110, 111, 100, 101, 115, 50, 48, 56, 58, 60, 56, 24, 135,
            184, 217, 159, 50, 227, 121, 127, 232, 134, 234, 230, 182, 87, 164, 197, 204, 132, 154,
            119, 10, 250, 203, 60, 56, 23, 214, 174, 82, 144, 73, 241, 241, 187, 233, 235, 179,
            166, 219, 60, 135, 12, 225, 120, 50, 67, 75, 172, 96, 60, 56, 23, 214, 174, 82, 144,
            73, 241, 241, 187, 233, 235, 179, 166, 219, 60, 135, 12, 225, 80, 2, 3, 75, 136, 188,
            60, 56, 22, 214, 174, 82, 144, 73, 241, 241, 187, 233, 235, 179, 166, 219, 60, 135, 12,
            225, 154, 68, 5, 156, 254, 122, 60, 56, 9, 43, 3, 30, 77, 129, 75, 232, 249, 57, 145,
            33, 147, 180, 112, 197, 64, 210, 92, 57, 190, 197, 26, 233, 60, 56, 11, 73, 241, 241,
            187, 233, 235, 179, 166, 219, 60, 135, 12, 62, 153, 36, 94, 82, 190, 47, 120, 18, 95,
            101, 60, 56, 13, 153, 90, 252, 17, 185, 200, 191, 213, 35, 70, 176, 175, 69, 89, 37,
            231, 13, 37, 23, 221, 159, 200, 213, 60, 56, 12, 19, 33, 244, 111, 119, 57, 178, 63,
            103, 254, 105, 52, 171, 176, 231, 54, 45, 106, 102, 11, 22, 182, 173, 53, 58, 116, 111,
            107, 101, 110, 50, 48, 58, 82, 215, 117, 175, 125, 94, 63, 180, 64, 184, 16, 253, 246,
            107, 39, 143, 99, 198, 60, 78, 54, 58, 118, 97, 108, 117, 101, 115, 108, 54, 58, 79,
            130, 127, 130, 33, 174, 54, 58, 151, 41, 225, 24, 33, 177, 54, 58, 109, 66, 33, 194,
            33, 173, 54, 58, 188, 163, 51, 130, 33, 173, 101, 101, 49, 58, 116, 49, 58, 13, 49, 58,
            118, 52, 58, 85, 84, 98, 22, 49, 58, 121, 49, 58, 114, 101,
        ];
        let (mut nodes, mut peers) = (Vec::new(), Vec::new());
        assert!(parse_response(&INPUT, |n| nodes.push(n), |p| peers.push(p)));
        assert_eq!(nodes.len(), 8);
        assert_eq!(nodes[0], "132.154.119.10:64203".parse().unwrap());
        assert_eq!(peers.len(), 4);
        assert_eq!(peers[0], "79.130.127.130:8622".parse().unwrap());
    }

    #[test]
    fn ignores_non_responses() {
        assert!(!parse_response(b"d1:y1:qe", |_| panic!(), |_| panic!()));
        assert!(!parse_response(b"garbage", |_| panic!(), |_| panic!()));
    }
}
