//! AceStream flavour of the BitTorrent peer protocol (download only).
//!
//! Threads:
//! * the caller's thread runs the DHT loop and publishes peer snapshots;
//! * `workers` threads each connect to a random peer and download pieces.
//!
//! Completed pieces go through a [`Gate`] that forwards them to the output in
//! order and without duplicates. Piece buffers are recycled through a small
//! pool, and every connection parses packets in place from a reusable receive
//! buffer, so the steady state performs no per-packet allocations and copies
//! each video byte exactly once (socket buffer -> piece buffer).

use std::convert::Infallible;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, SocketAddrV4, TcpStream};
use std::ops::Deref;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use crate::acestream::{api, torrent::Torrent};
use crate::bencode;
use crate::bittorrent::dht::{self, DhtManager};
use crate::utils::Rng;

const DHT_PORT: u16 = 8621;
const BITTORRENT_PORT: u16 = 8621;

// BitTorrent protocol message ids used by the client.
const CMD_INTERESTED: u8 = 2;
const CMD_REQUEST: u8 = 6;
const CMD_PIECE: u8 = 7;
const CMD_EXTEND_INFO: u8 = 0x14;

const PROTOCOL_NAME: &[u8; 17] = b"AceStreamProtocol";
const HANDSHAKE_LEN: usize = 46;
const PEER_ID_LEN: usize = 20;
const MAX_PACKET_LEN: i32 = 100_000;
/// piece index (8) + unknown (8) + chunk index (2)
const CHUNK_HEADER_LEN: usize = 18;
/// The last chunk of every piece ends with a signature that is not video.
const SIGNATURE_LEN: usize = 0x60;
/// length (4) + id (1) + piece (8) + chunk (2)
const REQUEST_LEN: usize = 15;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(2500);
const READ_TIMEOUT: Duration = Duration::from_millis(5000);
const WRITE_TIMEOUT: Duration = Duration::from_millis(4500);
const PIECE_RETRY_SECS: u64 = 3;
/// A peer further ahead than this (in pieces) resets the playback position.
const MAX_POSITION_LAG: i64 = 60;

const RECV_BUF_LEN: usize = 64 * 1024;
/// Compact the receive buffer when less than this is free at its end.
const MIN_READ_SPACE: usize = 16 * 1024;
const MAX_POOLED_BUFFERS: usize = 4;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A complete piece, ready to be sent to the player.
pub struct FullPiece {
    pub piece_idx: i32,
    pub data: PieceData,
}

/// Piece payload. Its buffer goes back to the pool when dropped.
pub struct PieceData {
    buf: Vec<u8>,
    pool: Arc<BufferPool>,
}

impl Deref for PieceData {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.buf
    }
}

impl Drop for PieceData {
    fn drop(&mut self) {
        self.pool.put(std::mem::take(&mut self.buf));
    }
}

struct BufferPool {
    free: Mutex<Vec<Vec<u8>>>,
    buf_capacity: usize,
}

impl BufferPool {
    fn take(&self) -> Vec<u8> {
        lock(&self.free)
            .pop()
            .unwrap_or_else(|| Vec::with_capacity(self.buf_capacity))
    }

    fn put(&self, mut buf: Vec<u8>) {
        buf.clear();
        let mut free = lock(&self.free);
        if free.len() < MAX_POOLED_BUFFERS {
            free.push(buf);
        }
    }
}

/// Serialises the hand-off of completed pieces: only the piece the output
/// expects next (or any piece before playback starts) is forwarded.
struct Gate {
    /// Next piece expected by the output, -1 before playback starts. Only
    /// written while holding `out`; workers read it lock-free.
    position: AtomicI64,
    out: Mutex<Option<Sender<FullPiece>>>,
    pool: Arc<BufferPool>,
}

impl Gate {
    fn position(&self) -> i64 {
        self.position.load(Ordering::Relaxed)
    }

    /// Called when a connection starts: adopts the peer position when
    /// playback has not started or lags too far behind. Returns `true` when
    /// the position was adopted.
    fn sync_start(&self, peer_position: i64) -> bool {
        let _guard = lock(&self.out);
        let current = self.position();
        if current == -1 || peer_position - current > MAX_POSITION_LAG {
            self.position.store(peer_position, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Forwards the piece in `buf` if it is the expected one. On success
    /// `buf` is replaced by an empty buffer from the pool.
    fn offer(&self, piece_idx: i32, buf: &mut Vec<u8>) {
        let out = lock(&self.out);
        let current = self.position();
        if current != -1 && piece_idx != current as i32 {
            return;
        }
        println!("[M] Received piece {} (len={})", piece_idx, buf.len());
        self.position.store(piece_idx as i64 + 1, Ordering::Relaxed);
        let data = PieceData {
            buf: std::mem::replace(buf, self.pool.take()),
            pool: self.pool.clone(),
        };
        if let Some(tx) = out.as_ref() {
            let _ = tx.send(FullPiece { piece_idx, data });
        }
    }
}

/// State shared by the DHT loop and all workers.
struct Shared {
    infohash: [u8; 20],
    piece_chunks: usize,
    peers: Mutex<Arc<Vec<SocketAddrV4>>>,
    gate: Gate,
}

pub struct BitTorrentClient {
    torrent: Torrent,
    dht: DhtManager,
    tx_out: Option<Sender<FullPiece>>,
}

pub fn new_client(torrent: Torrent, tx_out: Option<Sender<FullPiece>>) -> BitTorrentClient {
    BitTorrentClient {
        torrent,
        dht: dht::new_default(),
        tx_out,
    }
}

impl BitTorrentClient {
    /// Runs the DHT loop forever, starting the download workers as soon as
    /// enough peers are known. Only returns on a fatal setup error.
    pub fn start(&mut self, workers: usize, d_port: Option<u16>) {
        let Some(infohash) = self.torrent.infohash_bytes() else {
            println!("[BT] Error: invalid infohash '{}'", self.torrent.infohash);
            return;
        };
        if self.torrent.chunk_length == 0 {
            println!("[BT] Error: invalid chunk length");
            return;
        }
        if let Err(e) = self.dht.listen(d_port.unwrap_or(DHT_PORT), &infohash) {
            println!("[DHT] Error: {e}");
            return;
        }

        let shared = Arc::new(Shared {
            infohash,
            piece_chunks: self.torrent.piece_length / self.torrent.chunk_length,
            peers: Mutex::new(Arc::new(Vec::new())),
            gate: Gate {
                position: AtomicI64::new(-1),
                out: Mutex::new(self.tx_out.take()),
                pool: Arc::new(BufferPool {
                    free: Mutex::new(Vec::new()),
                    buf_capacity: self.torrent.piece_length,
                }),
            },
        });

        self.dht.get_peers();
        let mut old_peers_len = self.dht.peers_len();
        let mut old_nodes_len = self.dht.nodes_len();
        let mut started_workers = false;
        let mut i = 0usize;
        println!("[DHT] Updating peers..");
        loop {
            self.dht.recv_peers();
            let peers_len = self.dht.peers_len();
            let nodes_len = self.dht.nodes_len();
            if old_peers_len != peers_len || old_nodes_len != nodes_len {
                println!("[DHT] Peers: {peers_len} // Nodes: {nodes_len}");
            }
            if !started_workers && peers_len >= 2 {
                // The peer list is published below, so the workers start
                // after a short pause and spread over the peers found by
                // then instead of all hitting the first two.
                start_workers(&shared, workers);
                started_workers = true;
            } else if i >= nodes_len {
                if nodes_len > 10000 && old_peers_len > 3 {
                    thread::sleep(Duration::from_millis(10000));
                }
                self.dht.get_peers();
                i = 0;
            }
            if old_peers_len != peers_len {
                *lock(&shared.peers) = Arc::new(self.dht.peers());
                old_peers_len = peers_len;
            }
            old_nodes_len = nodes_len;
            i += 1;
        }
    }
}

fn start_workers(shared: &Arc<Shared>, workers: usize) {
    for n in 0..workers {
        let worker = Worker::new(shared.clone());
        if let Err(e) = thread::Builder::new()
            .name(format!("worker-{n}"))
            .spawn(move || worker.run())
        {
            println!("[W] ERROR: unable to start worker: {e}");
        }
    }
}

struct Worker {
    shared: Arc<Shared>,
    rng: Rng,
    /// Receive buffer, reused across connections.
    recv_buf: Vec<u8>,
    /// Piece being assembled.
    piece_buf: Vec<u8>,
    /// Serialised chunk requests of a piece.
    request_buf: Vec<u8>,
}

impl Worker {
    fn new(shared: Arc<Shared>) -> Self {
        let piece_buf = shared.gate.pool.take();
        let request_buf = Vec::with_capacity(shared.piece_chunks * REQUEST_LEN);
        Worker {
            shared,
            rng: Rng::new(),
            recv_buf: vec![0; RECV_BUF_LEN],
            piece_buf,
            request_buf,
        }
    }

    fn run(mut self) {
        loop {
            let peers = lock(&self.shared.peers).clone();
            if !peers.is_empty() {
                let peer = peers[self.rng.below(peers.len())];
                let Ok(stream) = TcpStream::connect_timeout(&SocketAddr::V4(peer), CONNECT_TIMEOUT)
                else {
                    continue;
                };
                let Err(e) = self.download(&stream, peer);
                println!("[W] ERROR: {e} ({peer})");
            }
            thread::sleep(Duration::from_millis(1000));
        }
    }

    /// Runs a download session with `peer`; it only ends on error.
    fn download(&mut self, stream: &TcpStream, peer: SocketAddrV4) -> Result<Infallible, String> {
        let Worker {
            shared,
            rng,
            recv_buf,
            piece_buf,
            request_buf,
        } = self;
        let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
        let _ = stream.set_nodelay(true);
        let mut conn = Connection {
            stream,
            buf: recv_buf,
            start: 0,
            end: 0,
        };

        // Handshake: 0x11 "AceStreamProtocol" 8 reserved bytes, infohash.
        let mut handshake = [0u8; HANDSHAKE_LEN];
        handshake[0] = PROTOCOL_NAME.len() as u8;
        handshake[1..18].copy_from_slice(PROTOCOL_NAME);
        handshake[26..].copy_from_slice(&shared.infohash);
        send(stream, &handshake)?;
        if conn.read_exact(HANDSHAKE_LEN)?[1..18] != PROTOCOL_NAME[..] {
            return Err("no_acestream_protocol".into());
        }
        send(stream, &generate_peer_id(rng))?;
        conn.read_exact(PEER_ID_LEN)?;

        // The peer announces its live position in its EXTEND_INFO message.
        let (id, payload) = conn
            .next_packet()
            .map_err(|e| format!("error_receiving_extend_info:{e}"))?;
        if id != CMD_EXTEND_INFO {
            return Err("error_extend_info_not_recv".into());
        }
        let mut actual_position = parse_max_piece(payload)?;
        println!("[W] Received actual position: {actual_position} (from: {peer})");

        // Our EXTEND_INFO and INTERESTED; the replies are not needed.
        send(stream, &extend_info_packet(peer.ip().octets()))?;
        let _ = conn.next_packet();
        send(stream, &[0, 0, 0, 1, CMD_INTERESTED])?;
        let _ = conn.next_packet();

        let piece_chunks = shared.piece_chunks;
        let gate = &shared.gate;
        let mut requested_piece = false;
        let mut requested_position = actual_position;
        let mut received_chunks = 0usize;
        let mut last_piece_time = Instant::now();
        piece_buf.clear();
        if gate.sync_start(actual_position) {
            encode_piece_request(request_buf, actual_position, piece_chunks);
            send(stream, request_buf)?;
        }

        loop {
            let (id, data) = conn
                .next_packet()
                .map_err(|e| format!("error_video_download:{e}"))?;
            if id == CMD_PIECE && data.len() >= CHUNK_HEADER_LEN {
                let piece_idx = i32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                let chunk_idx = i16::from_be_bytes([data[16], data[17]]);
                let chunk = &data[CHUNK_HEADER_LEN..];
                // Only accept the next chunk of the piece being assembled.
                if i64::from(chunk_idx) != received_chunks as i64
                    || piece_idx != requested_position as i32
                {
                    continue;
                }
                received_chunks += 1;
                if received_chunks < piece_chunks {
                    piece_buf.extend_from_slice(chunk);
                    continue;
                }
                // Last chunk: strip the signature from the video payload.
                let video_len = chunk
                    .len()
                    .checked_sub(SIGNATURE_LEN)
                    .ok_or("error_video_download:short_last_chunk")?;
                piece_buf.extend_from_slice(&chunk[..video_len]);
                println!("[W] Received full piece ({piece_idx}) from {peer}");
                gate.offer(piece_idx, piece_buf);
                actual_position += 1;
                requested_piece = false;
                received_chunks = 0;
            }

            if last_piece_time.elapsed().as_secs() > PIECE_RETRY_SECS {
                requested_piece = false;
                last_piece_time = Instant::now();
            }
            if !requested_piece {
                // Follow the output position unless it points at the piece
                // this worker has just completed.
                let client_position = gate.position();
                if client_position != -1 && client_position != actual_position - 1 {
                    actual_position = client_position;
                }
                encode_piece_request(request_buf, actual_position, piece_chunks);
                send(stream, request_buf)?;
                requested_position = actual_position;
                requested_piece = true;
                received_chunks = 0;
                piece_buf.clear();
            }
        }
    }
}

fn send(stream: &TcpStream, data: &[u8]) -> Result<(), String> {
    (&mut &*stream).write_all(data).map_err(|e| e.to_string())
}

/// Buffered reader that parses packets in place.
struct Connection<'a> {
    stream: &'a TcpStream,
    buf: &'a mut Vec<u8>,
    start: usize,
    end: usize,
}

impl Connection<'_> {
    /// Ensures at least `n` bytes are buffered.
    fn fill(&mut self, n: usize) -> Result<(), String> {
        while self.end - self.start < n {
            if self.start == self.end {
                self.start = 0;
                self.end = 0;
            } else if self.start > 0
                && (self.buf.len() - self.start < n || self.buf.len() - self.end < MIN_READ_SPACE)
            {
                self.buf.copy_within(self.start..self.end, 0);
                self.end -= self.start;
                self.start = 0;
            }
            if self.buf.len() - self.start < n {
                self.buf.resize(self.start + n, 0);
            }
            match (&mut &*self.stream).read(&mut self.buf[self.end..]) {
                Ok(0) => return Err("nothing_received".into()),
                Ok(read) => self.end += read,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(())
    }

    fn read_exact(&mut self, n: usize) -> Result<&[u8], String> {
        self.fill(n)?;
        let start = self.start;
        self.start += n;
        Ok(&self.buf[start..start + n])
    }

    /// Returns the next packet as `(message id, payload)`. Keep-alive
    /// packets are reported with id 0 and an empty payload.
    fn next_packet(&mut self) -> Result<(u8, &[u8]), String> {
        self.fill(4)?;
        let s = self.start;
        let len = i32::from_be_bytes([
            self.buf[s],
            self.buf[s + 1],
            self.buf[s + 2],
            self.buf[s + 3],
        ]);
        if !(0..=MAX_PACKET_LEN).contains(&len) {
            return Err(format!("bad_packet_len:{len}"));
        }
        if len == 0 {
            self.start += 4;
            return Ok((0, &[]));
        }
        let packet = self.read_exact(4 + len as usize)?;
        Ok((packet[4], &packet[5..]))
    }
}

/// Serialises the REQUEST messages for every chunk of `piece` so they can be
/// sent with a single write.
fn encode_piece_request(buf: &mut Vec<u8>, piece: i64, chunks: usize) {
    buf.clear();
    for chunk in 0..chunks {
        buf.extend_from_slice(&11u32.to_be_bytes());
        buf.push(CMD_REQUEST);
        buf.extend_from_slice(&piece.to_be_bytes());
        buf.extend_from_slice(&(chunk as i16).to_be_bytes());
    }
}

fn generate_peer_id(rng: &mut Rng) -> [u8; PEER_ID_LEN] {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut id = *b"R20------00000000000";
    for b in &mut id[9..] {
        *b = ALPHABET[rng.below(ALPHABET.len())];
    }
    id
}

/// Extracts `mi.max_piece` from the peer's EXTEND_INFO payload.
fn parse_max_piece(payload: &[u8]) -> Result<i64, String> {
    // The first payload byte is the extended message type.
    let body = payload
        .get(1..)
        .ok_or("error_bdecoding_extend_info:empty_payload")?;
    let info = bencode::decode(body).map_err(|e| format!("error_bdecoding_extend_info:{e}"))?;
    info.get(b"mi")
        .and_then(|mi| mi.get(b"max_piece"))
        .and_then(|v| v.as_int())
        .ok_or_else(|| "error_bdecoding_extend_info:missing field `max_piece`".into())
}

/// Builds our initial EXTEND_INFO packet (bencoded dictionary with sorted
/// keys, preceded by the extended message type 0).
fn extend_info_packet(peer_ip: [u8; 4]) -> Vec<u8> {
    let version_code: i64 = api::VERSION_CODE.parse().unwrap_or(3013200);
    let mut p = Vec::with_capacity(300);
    p.extend_from_slice(&[0, 0, 0, 0, CMD_EXTEND_INFO, 0]); // length is patched below
    p.push(b'd');
    bencode::put_key_int(&mut p, "lsp", 284912);
    bencode::put_bytes(&mut p, b"mi");
    p.push(b'd');
    bencode::put_key_int(&mut p, "down_rate", 80000);
    bencode::put_key_int(&mut p, "is_accessible", 0);
    bencode::put_key_int(&mut p, "live_window_size", 987);
    bencode::put_key_int(&mut p, "max_piece", -1);
    bencode::put_key_int(&mut p, "min_piece", -1);
    bencode::put_key_int(&mut p, "position", -1);
    bencode::put_key_int(&mut p, "top_session_up_rate", 0);
    bencode::put_key_int(&mut p, "top_up_rate", 0);
    bencode::put_key_int(&mut p, "up_rate", 0);
    bencode::put_key_int(&mut p, "upload_rating", 0);
    p.push(b'e');
    bencode::put_key_int(&mut p, "nt", 1);
    bencode::put_key_int(&mut p, "opts", 15);
    bencode::put_key_int(&mut p, "p", BITTORRENT_PORT as i64);
    bencode::put_key_int(&mut p, "pv", 2);
    bencode::put_key_int(&mut p, "ts", 413);
    bencode::put_bytes(&mut p, b"tt");
    bencode::put_bytes(&mut p, b"bt");
    bencode::put_key_int(&mut p, "v", version_code);
    bencode::put_bytes(&mut p, b"yourip");
    bencode::put_bytes(&mut p, &peer_ip);
    p.push(b'e');
    let len = (p.len() - 4) as u32;
    p[..4].copy_from_slice(&len.to_be_bytes());
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn extend_info_matches_original_encoding() {
        // Packet produced by the original serde_bencode based implementation.
        let expected: &[u8] = &[
            0, 0, 1, 21, 20, 0, 100, 51, 58, 108, 115, 112, 105, 50, 56, 52, 57, 49, 50, 101, 50,
            58, 109, 105, 100, 57, 58, 100, 111, 119, 110, 95, 114, 97, 116, 101, 105, 56, 48, 48,
            48, 48, 101, 49, 51, 58, 105, 115, 95, 97, 99, 99, 101, 115, 115, 105, 98, 108, 101,
            105, 48, 101, 49, 54, 58, 108, 105, 118, 101, 95, 119, 105, 110, 100, 111, 119, 95,
            115, 105, 122, 101, 105, 57, 56, 55, 101, 57, 58, 109, 97, 120, 95, 112, 105, 101, 99,
            101, 105, 45, 49, 101, 57, 58, 109, 105, 110, 95, 112, 105, 101, 99, 101, 105, 45, 49,
            101, 56, 58, 112, 111, 115, 105, 116, 105, 111, 110, 105, 45, 49, 101, 49, 57, 58, 116,
            111, 112, 95, 115, 101, 115, 115, 105, 111, 110, 95, 117, 112, 95, 114, 97, 116, 101,
            105, 48, 101, 49, 49, 58, 116, 111, 112, 95, 117, 112, 95, 114, 97, 116, 101, 105, 48,
            101, 55, 58, 117, 112, 95, 114, 97, 116, 101, 105, 48, 101, 49, 51, 58, 117, 112, 108,
            111, 97, 100, 95, 114, 97, 116, 105, 110, 103, 105, 48, 101, 101, 50, 58, 110, 116,
            105, 49, 101, 52, 58, 111, 112, 116, 115, 105, 49, 53, 101, 49, 58, 112, 105, 56, 54,
            50, 49, 101, 50, 58, 112, 118, 105, 50, 101, 50, 58, 116, 115, 105, 52, 49, 51, 101,
            50, 58, 116, 116, 50, 58, 98, 116, 49, 58, 118, 105, 51, 48, 49, 51, 50, 48, 48, 101,
            54, 58, 121, 111, 117, 114, 105, 112, 52, 58, 87, 219, 33, 178, 101,
        ];
        assert_eq!(extend_info_packet([87, 219, 33, 178]), expected);
    }

    #[test]
    fn parses_peer_extend_info() {
        const INPUT: [u8; 519] = [
            100, 50, 48, 58, 97, 99, 101, 95, 109, 101, 116, 97, 100, 97, 116, 97, 95, 118, 101,
            114, 115, 105, 111, 110, 105, 49, 101, 49, 51, 58, 103, 101, 111, 105, 112, 95, 99,
            111, 117, 110, 116, 114, 121, 50, 58, 69, 83, 51, 58, 108, 115, 112, 105, 49, 49, 53,
            51, 50, 56, 101, 49, 58, 109, 100, 49, 49, 58, 117, 116, 95, 109, 101, 116, 97, 100,
            97, 116, 97, 105, 50, 101, 101, 50, 58, 109, 105, 100, 50, 48, 58, 100, 105, 115, 116,
            97, 110, 99, 101, 95, 102, 114, 111, 109, 95, 115, 111, 117, 114, 99, 101, 105, 48,
            101, 57, 58, 100, 111, 119, 110, 95, 114, 97, 116, 101, 105, 53, 57, 50, 49, 55, 49,
            101, 49, 57, 58, 100, 111, 119, 110, 108, 111, 97, 100, 95, 119, 105, 110, 100, 111,
            119, 95, 101, 110, 100, 105, 49, 49, 53, 51, 50, 56, 101, 49, 51, 58, 105, 115, 95, 97,
            99, 99, 101, 115, 115, 105, 98, 108, 101, 105, 49, 101, 49, 54, 58, 108, 105, 118, 101,
            95, 119, 105, 110, 100, 111, 119, 95, 115, 105, 122, 101, 105, 49, 49, 48, 101, 51, 58,
            108, 115, 112, 105, 49, 49, 53, 51, 50, 56, 101, 51, 58, 109, 97, 109, 105, 45, 49,
            101, 57, 58, 109, 97, 120, 95, 112, 105, 101, 99, 101, 105, 49, 49, 53, 51, 50, 55,
            101, 57, 58, 109, 105, 110, 95, 112, 105, 101, 99, 101, 105, 49, 49, 52, 55, 51, 50,
            101, 57, 58, 112, 101, 101, 114, 95, 116, 121, 112, 101, 105, 48, 101, 49, 54, 58, 112,
            105, 110, 103, 95, 102, 114, 111, 109, 95, 115, 111, 117, 114, 99, 101, 105, 48, 101,
            56, 58, 112, 111, 115, 105, 116, 105, 111, 110, 105, 49, 49, 53, 51, 49, 54, 101, 49,
            54, 58, 116, 105, 109, 101, 95, 102, 114, 111, 109, 95, 115, 111, 117, 114, 99, 101,
            105, 45, 49, 101, 49, 57, 58, 116, 111, 112, 95, 115, 101, 115, 115, 105, 111, 110, 95,
            117, 112, 95, 114, 97, 116, 101, 105, 50, 50, 54, 53, 57, 48, 53, 101, 49, 49, 58, 116,
            111, 112, 95, 117, 112, 95, 114, 97, 116, 101, 105, 50, 50, 51, 55, 55, 51, 54, 101,
            55, 58, 117, 112, 95, 114, 97, 116, 101, 105, 52, 53, 49, 55, 53, 101, 49, 51, 58, 117,
            112, 108, 111, 97, 100, 95, 114, 97, 116, 105, 110, 103, 105, 48, 101, 101, 50, 58,
            110, 116, 105, 49, 101, 49, 58, 112, 105, 56, 54, 50, 49, 101, 56, 58, 112, 108, 97,
            116, 102, 111, 114, 109, 105, 51, 101, 50, 58, 112, 118, 105, 50, 101, 50, 58, 116,
            115, 105, 49, 54, 51, 50, 57, 49, 55, 49, 52, 55, 101, 50, 58, 116, 116, 50, 58, 98,
            116, 49, 58, 118, 105, 51, 48, 49, 54, 55, 48, 49, 101, 54, 58, 121, 111, 117, 114,
            105, 112, 52, 58, 87, 219, 33, 178, 101,
        ];
        let mut payload = vec![0u8];
        payload.extend_from_slice(&INPUT);
        assert_eq!(parse_max_piece(&payload), Ok(115327));
        assert!(parse_max_piece(&[]).is_err());
        assert!(parse_max_piece(&[0, b'd', b'e']).is_err());
    }

    #[test]
    fn piece_request_layout() {
        let mut buf = Vec::new();
        encode_piece_request(&mut buf, 275228605, 32);
        assert_eq!(buf.len(), 32 * REQUEST_LEN);
        assert_eq!(
            &buf[..15],
            &[0, 0, 0, 11, 6, 0, 0, 0, 0, 16, 103, 167, 189, 0, 0]
        );
        assert_eq!(&buf[buf.len() - 2..], &[0, 31]);
    }

    #[test]
    fn peer_id_format() {
        let id = generate_peer_id(&mut Rng::new());
        assert!(id.starts_with(b"R20------"));
        assert!(id[9..].iter().all(u8::is_ascii_alphanumeric));
    }

    /// Feeds packets split at awkward boundaries through a real socket.
    #[test]
    fn connection_reassembles_fragmented_packets() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let writer = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut data = Vec::new();
            data.extend_from_slice(b"HELLO");
            data.extend_from_slice(&[0, 0, 0, 0]); // keep-alive
            for i in 0..50u32 {
                let len = 1 + 1000 * (i as usize % 7);
                data.extend_from_slice(&(len as u32).to_be_bytes());
                data.push(i as u8);
                data.extend(std::iter::repeat_n(i as u8, len - 1));
            }
            for piece in data.chunks(777) {
                s.write_all(piece).unwrap();
                s.flush().unwrap();
            }
        });

        let stream = TcpStream::connect(addr).unwrap();
        let mut buf = vec![0; 4096]; // small on purpose: forces compaction and growth
        let mut conn = Connection {
            stream: &stream,
            buf: &mut buf,
            start: 0,
            end: 0,
        };
        assert_eq!(conn.read_exact(5).unwrap(), b"HELLO");
        assert_eq!(conn.next_packet().unwrap(), (0, &[][..]));
        for i in 0..50u32 {
            let (id, payload) = conn.next_packet().unwrap();
            assert_eq!(id, i as u8);
            assert_eq!(payload.len(), 1000 * (i as usize % 7));
            assert!(payload.iter().all(|&b| b == i as u8));
        }
        writer.join().unwrap();
        assert_eq!(conn.next_packet().unwrap_err(), "nothing_received");
    }

    #[test]
    fn rejects_bad_packet_lengths() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let writer = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(&(-1i32).to_be_bytes()).unwrap();
        });
        let stream = TcpStream::connect(addr).unwrap();
        let mut buf = vec![0; 64];
        let mut conn = Connection {
            stream: &stream,
            buf: &mut buf,
            start: 0,
            end: 0,
        };
        assert_eq!(conn.next_packet().unwrap_err(), "bad_packet_len:-1");
        writer.join().unwrap();
    }

    #[test]
    fn gate_forwards_in_order() {
        let (tx, rx) = std::sync::mpsc::channel();
        let pool = Arc::new(BufferPool {
            free: Mutex::new(Vec::new()),
            buf_capacity: 16,
        });
        let gate = Gate {
            position: AtomicI64::new(-1),
            out: Mutex::new(Some(tx)),
            pool: pool.clone(),
        };

        let mut buf = vec![1, 2, 3];
        gate.offer(10, &mut buf); // first piece is always accepted
        assert!(buf.is_empty());
        let mut stale = vec![9];
        gate.offer(10, &mut stale); // duplicate: rejected, buffer untouched
        assert_eq!(stale, [9]);
        let mut next = vec![4];
        gate.offer(11, &mut next);
        assert_eq!(gate.position(), 12);

        let received: Vec<(i32, Vec<u8>)> = rx
            .try_iter()
            .map(|p| (p.piece_idx, p.data.to_vec()))
            .collect();
        assert_eq!(received, [(10, vec![1, 2, 3]), (11, vec![4])]);
        // Dropped pieces returned their buffers to the pool.
        assert_eq!(lock(&pool.free).len(), 2);

        assert!(!gate.sync_start(40)); // close enough: keep the position
        assert!(gate.sync_start(100)); // too far ahead: jump
        assert_eq!(gate.position(), 100);
    }
}
