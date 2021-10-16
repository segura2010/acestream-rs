
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::sync::mpsc::{Sender, Receiver};
use std::net::{TcpStream, SocketAddr};
use byteorder::{BigEndian, WriteBytesExt, ReadBytesExt};
use std::io::{Write, Read, Cursor};

use rand::prelude::*;
use rand::seq::SliceRandom;
use rand::distributions::Slice;

use serde::{Serialize, Deserialize};
use serde_bencode::de::from_bytes;
use serde_bencode::ser::to_bytes;
use serde_bytes::ByteBuf;

use crate::acestream::torrent;
use crate::acestream::api;
use crate::bittorrent::dht;

const DHT_PORT: u16 = 8621;
const BITTORRENT_PORT: u16 = 8621;

// BitTorrent Protocol Messages IDs
const CMD_CHOKE: u8 = 0;
const CMD_UNCHOKE: u8 = 1;
const CMD_INTERESTED: u8 = 2;
const CMD_NOT_INTERESTED: u8 = 3;
const CMD_HAVE: u8 = 4;
const CMD_BITFIELD: u8 = 5;
const CMD_REQUEST: u8 = 6;
const CMD_PIECE: u8 = 7;
const CMD_CANCEL: u8 = 8;
const CMD_PORT: u8 = 9;
const CMD_EXTEND_INFO_11: u8 = 11;
const CMD_EXTEND_INFO: u8 = 0x14;

pub struct BitTorrentClient {
    torrent: torrent::Torrent,
    dht: dht::DHTManager,
    peers: Arc<Mutex<Vec<dht::Peer>>>,
    started_workers: bool,

    actual_position: Arc<Mutex<i64>>,
    
    tx: Option<Sender<FullPiece>>,
    tx_out: Option<Sender<FullPiece>>,
}

pub fn new_client(t: torrent::Torrent, tx_out: Option<Sender<FullPiece>>) -> BitTorrentClient {
    BitTorrentClient{
        torrent: t,
        dht: dht::new_default(),
        peers: Arc::new(Mutex::new(Vec::with_capacity(30))),
        started_workers: false,
        actual_position: Arc::new(Mutex::new(-1)),
        tx: None,
        tx_out: tx_out,
    }
}

impl BitTorrentClient {
    pub fn start(&mut self, workers: usize) {
        let infohash = hex::decode(self.torrent.infohash.clone()).unwrap();
        self.dht.infohash = infohash.clone();
        self.dht.listen(format!("0.0.0.0:{}", DHT_PORT));

        self.dht.get_peers(&infohash);
        let mut old_peers_len = self.dht.peers.len();
        let mut old_nodes_len = self.dht.nodes.len();
        let mut i = 0;
        println!("Updating peers..");
        loop{
            self.dht.recv_peers();
            if old_peers_len != self.dht.peers.len() || old_nodes_len != self.dht.nodes.len() {
                println!("Peers: {} // Nodes: {}", self.dht.peers.len(), self.dht.nodes.len());
            }
            if !self.started_workers && self.dht.peers.len() >= 2 {
                self.start_manager_worker();
                self.start_workers(workers);
                self.started_workers = true;
            } else if i == self.dht.nodes.len() {
                if self.dht.nodes.len() > 10000 && old_peers_len > 3 {
                    std::thread::sleep(std::time::Duration::from_millis(10000));
                }
                self.dht.get_peers(&infohash);
                i = 0;
            }
            if old_peers_len != self.dht.peers.len() {
                self.peers.lock().unwrap().clear();
                for p in self.dht.peers.iter() {
                    self.peers.lock().unwrap().push(*p);
                }
                old_peers_len = self.dht.peers.len();
            }

            old_nodes_len = self.dht.nodes.len();
            i += 1;
        }
    }

    fn start_workers(&mut self, workers: usize) {
        for i in 0..workers {
            let shared_tx = self.tx.as_ref().unwrap().clone();
            let shared_peers = self.peers.clone();
            let torrent = self.torrent.clone();
            let shared_act_pos = self.actual_position.clone();

            std::thread::spawn(move||{
                let mut connector = new_connector(torrent, shared_peers, shared_tx, shared_act_pos);
                connector.start();
            });
        }
    }

    fn start_manager_worker(&mut self) {
        let torrent = self.torrent.clone();
        let act_pos = self.actual_position.clone();
        let tx_out = self.tx_out.clone();

        let (tx, rx) = mpsc::channel();
        self.tx = Some(tx);

        std::thread::spawn(move||{
            let mut act_pos_ = -1;
            loop{
                match rx.recv() {
                    Ok(piece) => {
                        {
                            act_pos_ = *(act_pos.lock().unwrap());
                        }
                        if piece.piece_idx == (act_pos_ as i32) || act_pos_ == -1 {
                            println!("[M] Received piece {} (len={})", piece.piece_idx, piece.data.len());
                            {
                                *(act_pos.lock().unwrap()) = (piece.piece_idx as i64) + 1;
                            }
                            if let Some(ref tx_out) = tx_out {
                                tx_out.send(piece);
                            }
                        }
                    },
                    Err(e) => { println!("[M] ERROR: {}", e) },
                }
            }
        });
    }
}

#[derive(Debug)]
pub struct BitTorrentPacket {
    length: i32,
    id: u8,
    data: Option<Vec<u8>>,
}
fn packet_from_header(header: Vec<u8>) -> BitTorrentPacket {
    let mut cursor = Cursor::new(header);
    BitTorrentPacket{
        length: cursor.read_i32::<BigEndian>().unwrap(),
        id: cursor.read_u8().unwrap(),
        data: None,
    }
}
impl BitTorrentPacket {
    fn as_bytes(&self) -> Vec<u8> {
        if let Some(data) = &self.data {
            let mut packet = Vec::with_capacity(data.len() + 4 + 1); // len + cmd + payload.len()
            packet.write_i32::<BigEndian>((data.len() + 1) as _); // cmd + payload.len()
            packet.write_u8(self.id);
            packet.write(&data);
            return packet;
        } else {
            let mut packet = Vec::with_capacity(5);
            packet.write_i32::<BigEndian>(1); // cmd + payload.len()
            packet.write_u8(self.id);
            return packet;
        }
    }
}

#[derive(Debug, Clone)]
pub struct FullPiece {
    pub piece_idx: i32,
    pub data: Vec<u8>,
}
pub struct Piece {
    unknown1: i32,
    piece_idx: i32,
    unknown2: i64,
    chunk_idx: i16,
    data: Vec<u8>,
}
fn new_piece_from_bytes(data: Vec<u8>, piece_len: usize) -> Piece {
    let mut cursor = Cursor::new(data);
    let mut p = Piece{
        unknown1: cursor.read_i32::<BigEndian>().unwrap(),
        piece_idx: cursor.read_i32::<BigEndian>().unwrap(),
        unknown2: cursor.read_i64::<BigEndian>().unwrap(),
        chunk_idx: cursor.read_i16::<BigEndian>().unwrap(),
        data: vec![0u8; piece_len],
    };
    cursor.read(&mut p.data);
    p
}

pub struct PeerConnector {
    torrent: torrent::Torrent,
    peers: Arc<Mutex<Vec<dht::Peer>>>,
    send_chan: Sender<FullPiece>,
    actual_peer: Option<dht::Peer>,

    actual_position: i64,
    actual_position_client: Arc<Mutex<i64>>,

    socket: Option<TcpStream>,
}

pub fn new_connector(t: torrent::Torrent, peers: Arc<Mutex<Vec<dht::Peer>>>, chan: Sender<FullPiece>, act_pos: Arc<Mutex<i64>>) -> PeerConnector {
    PeerConnector{
        torrent: t,
        peers: peers,
        send_chan: chan,
        actual_peer: None,
        actual_position: -1,
        actual_position_client: act_pos,
        socket: None,
    }
}

impl PeerConnector {
    pub fn start(&mut self) {
        let mut rng = rand::thread_rng();
        loop{
            let peers: Vec<dht::Peer>;
            {
                peers = self.peers.lock().unwrap().clone();
            }
            if let Some(p) = peers.choose(&mut rng) {
                self.actual_peer = Some(*p);
                //println!("Worker selected peer: {:?}", self.actual_peer);
                if let Ok(_) = self.connect(self.actual_peer.unwrap()) {
                    match self.start_download() {
                        Ok(_) => { println!("[W] Download finished for some reason: {:?}", self.actual_peer); }
                        Err(e) => { println!("[W] ERROR: {} ({:?})", e, self.actual_peer); }
                    }
                } else {
                    // TODO: maybe flag?
                    continue;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1000));
        }
    }

    fn start_download(&mut self) -> Result<usize, String> {
        self.send_handshake();
        let handshake = self.recv_handshake()?;
        if let Ok(client) = String::from_utf8(handshake[1..18].to_vec()) {
            if client.ne("AceStreamProtocol") {
                return Err("no_acestream_protocol".to_string());
            }
        }
        self.send_peer_id();
        self.recv_peer_id();

        // receive EXTEND_INFO
        match self.recv() {
            Ok(pkt) => {
                if pkt.id == CMD_EXTEND_INFO {
                    match from_bytes::<InitialExtendInfo>(&pkt.data.unwrap()[1..]) {
                        Ok(info) => {
                            self.actual_position = info.mi.max_piece;
                            println!("[W] Received actual position: {} (from: {:?})", self.actual_position, self.actual_peer);
                        },
                        Err(e) => {
                            return Err(format!("error_bdecoding_extend_info:{}", e).to_string());
                        },
                    }
                } else {
                    return Err("error_extend_info_not_recv".to_string());
                }
            },
            Err(e) => {
                return Err(format!("error_receiving_extend_info:{}", e).to_string());
            },
        }

        // send my initial extend_info
        {
            let packet = new_initial_extend_info_packet(&self.actual_peer.unwrap().ip);
            self.send(packet);
            self.recv();
        }
        // send interested
        {
            let packet = BitTorrentPacket{
                id: CMD_INTERESTED,
                length: 1,
                data: None,
            };
            self.send(packet);
            self.recv();
        }

        // some set up to start the pieces download
        let mut last_piece = FullPiece{
            piece_idx: 0,
            data: Vec::with_capacity(self.torrent.piece_length - 0x60), // -0x60 -> signature in the last chunk is 0x60 bytes long
        };
        let mut last_piece_time = std::time::Instant::now();
        let piece_chunks = self.torrent.piece_length / self.torrent.chunk_length;
        let mut received_chunks = 0;
        let mut requested_piece = false;
        // update bittorrent client position if it is very old or -1 (not started)
        {
            let mut client_pos = self.actual_position_client.lock().unwrap();
            let very_old_pos = (self.actual_position - *client_pos) > 60;
            if *client_pos == -1 || very_old_pos {
                *client_pos = self.actual_position;
                self.request_piece(self.actual_position);
            }
        }

        loop{
            match self.recv() {
                Ok(packet) => {
                    match packet.id {
                        CMD_PIECE => {
                            if let Some(packet_data) = packet.data {
                                let piece = new_piece_from_bytes(packet_data.clone(), packet_data.len() - 18);
                                if piece.chunk_idx != received_chunks {
                                    continue;
                                }
                                received_chunks += 1;
                                if received_chunks >= (piece_chunks as _) {
                                    // last chunk, we must remove the signature from the video payload
                                    println!("[W] Received full piece ({}) from {:?}", piece.piece_idx, self.actual_peer);
                                    last_piece.data.extend_from_slice(&piece.data[..piece.data.len()-0x60]);
                                    last_piece.piece_idx = piece.piece_idx;
                                    self.send_chan.send(last_piece.clone());
                                    self.actual_position += 1;
                                    requested_piece = false;
                                    received_chunks = 0;
                                } else {
                                    //println!("Received chunk {}/{} (piece:{}) from {:?} (chunklen={})", piece.chunk_idx, piece_chunks, piece.piece_idx, self.actual_peer, piece.data.len());
                                    last_piece.data.extend_from_slice(&piece.data);
                                    continue;
                                }
                            }
                        },
                        CMD_EXTEND_INFO_11 => {
                        },
                        _ => {
                        },
                    }

                    let elapsed_since_last_piece = last_piece_time.elapsed().as_secs();
                    if elapsed_since_last_piece > 3 {
                        requested_piece = false;
                        last_piece_time = std::time::Instant::now();
                    }
                    if !requested_piece {
                        {
                            let act_pos_client = self.actual_position_client.lock().unwrap();
                            let old_piece = *act_pos_client == (self.actual_position-1);
                            if *act_pos_client != -1 && !old_piece {
                                self.actual_position = *act_pos_client;
                            }
                        }
                        self.request_piece(self.actual_position);
                        requested_piece = true;
                        received_chunks = 0;
                        last_piece.data.clear();
                    }
                },
                Err(e) => {
                    return Err(format!("error_video_download:{}", e.to_string()).to_string());
                },
            }
        }
    }

    fn connect(&mut self, peer: dht::Peer) -> Result<usize, String> {
        let conn = TcpStream::connect_timeout(&SocketAddr::from((peer.ip, peer.port)), 
                    std::time::Duration::from_millis(2500));
        match conn {
            Ok(s) => {
                s.set_read_timeout(Some(std::time::Duration::from_millis(5000)));
                s.set_write_timeout(Some(std::time::Duration::from_millis(4500)));
                self.socket = Some(s);
                return Ok(1);
            },
            Err(e) => {
                return Err(e.to_string());
            },
        }
    }

    fn recv(&self) -> Result<BitTorrentPacket, String> {
        if let Some(mut socket) = self.socket.as_ref() {
            let mut header = [0u8; 5]; // 4 bytes for length + 1 for message ID
            match socket.read(&mut header[..4]) {
                Ok(n) => {
                    if n <= 0 {
                        return Err("nothing_received".to_string());
                    }
                    let mut packet = packet_from_header(header.to_vec());
                    if packet.length > 100000 || packet.length < 0 {
                        return Err(format!("bad_packet_len:{}", packet.length).to_string());
                    } else if packet.length == 0 {
                        // ping packet
                        return Ok(packet);
                    }
                    const BUF_SIZE: usize = 5000;
                    let mut buff = [0u8; BUF_SIZE];
                    let mut payload = Vec::with_capacity(packet.length as _);
                    let mut read_bytes = 0usize;
                    let mut diff = (packet.length as usize) - read_bytes;
                    if diff > BUF_SIZE {
                        diff = BUF_SIZE;
                    }
                    loop {
                        if read_bytes >= (packet.length as _){
                            break;
                        }
                        match socket.read(&mut buff[..diff]) {
                            Ok(n) => {
                                read_bytes += n;
                                diff = (packet.length as usize) - read_bytes;
                                if diff > BUF_SIZE {
                                    diff = BUF_SIZE;
                                }
                                payload.extend_from_slice(&buff[0..n]);
                                continue;
                            },
                            Err(e) => {
                                return Err(e.to_string());
                            },
                        }
                    }
                    packet.id = payload[0];
                    packet.data = Some(payload[1..].to_vec());
                    return Ok(packet);
                },
                Err(e) => {
                    return Err(e.to_string());
                },
            }
        }
        Err("error_recieving_packet".to_string())
    }

    fn send(&self, packet: BitTorrentPacket) -> Result<usize, String> {
        if let Some(mut s) = self.socket.as_ref() {
            let result = s.write(&packet.as_bytes());
            match result {
                Ok(n) => { return Ok(n) },
                Err(e) => { return Err(e.to_string()) },
            }
        }
        Err("no_tcpsocket".to_string())
    }

    fn send_handshake(&self) -> Result<usize, String>{
        if let Some(mut s) = self.socket.as_ref() {
            let mut payload: Vec<u8> = Vec::with_capacity(46);

            payload.write_u8(0x11);
            payload.write(b"AceStreamProtocol");
            payload.write_i64::<BigEndian>(0);
            payload.write(&hex::decode(self.torrent.infohash.clone()).unwrap());

            let result = s.write(&payload);
            match result {
                Ok(n) => { return Ok(n) },
                Err(e) => { return Err(e.to_string()) },
            }
        }
        Err("no_tcpsocket".to_string())
    }
    fn recv_handshake(&self) -> Result<Vec<u8>, String>{
        if let Some(mut s) = self.socket.as_ref() {
            let mut payload = [0u8; 46];
            let result = s.read(&mut payload);
            match result {
                Ok(n) => { return Ok(payload.to_vec()) },
                Err(e) => { return Err(e.to_string()) },
            }
        }
        Err("no_tcpsocket".to_string())
    }

    fn send_peer_id(&self) -> Result<usize, String>{
        if let Some(mut s) = self.socket.as_ref() {
            let payload = format!("R20------{}", generate_peer_id(11));

            let result = s.write(payload.as_bytes());
            match result {
                Ok(n) => { return Ok(n) },
                Err(e) => { return Err(e.to_string()) },
            }
        }
        Err("no_tcpsocket".to_string())
    }
    fn recv_peer_id(&self) -> Result<Vec<u8>, String>{
        if let Some(mut s) = self.socket.as_ref() {
            let mut payload = [0u8; 20];
            let result = s.read(&mut payload);
            match result {
                Ok(n) => { return Ok(payload.to_vec()) },
                Err(e) => { return Err(e.to_string()) },
            }
        }
        Err("no_tcpsocket".to_string())
    }

    fn request_piece_chunk(&self, piece: i64, chunk: i16) -> Result<usize, String>{
        let mut payload: Vec<u8> = Vec::with_capacity(10);
        payload.write_i64::<BigEndian>(piece);
        payload.write_i16::<BigEndian>(chunk);
        let mut packet = BitTorrentPacket{
            length: 11,
            id: CMD_REQUEST as _,
            data: Some(payload),
        };
        self.send(packet)
    }
    fn request_piece(&self, piece: i64) {
        let piece_chunks = self.torrent.piece_length / self.torrent.chunk_length;
        for i in 0..piece_chunks {
            self.request_piece_chunk(piece, i as _);
        }
    }
}

fn generate_peer_id(size: usize) -> String {
    let alphabet = Slice::new(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789").unwrap();
    let rng = rand::thread_rng();
    let peer_id: Vec<u8> = rng.sample_iter(&alphabet).take(size).map(|b| { *b }).collect();
    String::from_utf8(peer_id).unwrap()
}

// BitTorrent BENCODE messages
#[derive(Debug, Serialize, Deserialize)]
struct InitialExtendInfo {
    pv: i64,
    tt: String,
    mi: InitialExtendInfoMI,
    ts: i64,
    p: i64,
    lsp: i64,
    v: i64,
    nt: i64,
    opts: Option<i64>,
    yourip: ByteBuf,
}
#[derive(Debug, Serialize, Deserialize)]
struct InitialExtendInfoMI {
    down_rate: i64,
    min_piece: i64,
    live_window_size: i64,
    is_accessible: i32,
    top_up_rate: i64,
    top_session_up_rate: i64,
    max_piece: i64,
    position: i64,
    up_rate: i64,
    upload_rating: i64,
}

impl InitialExtendInfo {
    pub fn to_bencode(&self) -> Result<Vec<u8>, String> {
        let encoded = to_bytes(self);
        if let Err(e) = encoded {
            return Err(format!("error_bencoding:{}", e).to_string())
        }
        return Ok(encoded.unwrap());
    }
}

fn new_initial_extend_info_packet(peer_addr: &[u8]) -> BitTorrentPacket {
    let mut version_code: i64 = 3013200;
    if let Ok(v_code) = api::VERSION_CODE.parse() {
        version_code = v_code;
    }

    let payload = InitialExtendInfo{
        pv: 2,
        tt: "bt".to_string(),
        mi: InitialExtendInfoMI{
            down_rate: 80000,
            min_piece: -1,
            live_window_size: 987,
            is_accessible: 0,
            top_up_rate: 0,
            top_session_up_rate: 0,
            max_piece: -1,
            position: -1,
            up_rate: 0,
            upload_rating: 0,
        },
        ts: 413,
        p: BITTORRENT_PORT as _,
        lsp: 284912,
        v: version_code,
        nt: 1,
        opts: Some(15),
        yourip: ByteBuf::from(peer_addr.to_vec()),
    };

    let mut final_payload = vec![0]; // extend_info type is one byte !!
    final_payload.extend_from_slice(&payload.to_bencode().unwrap());

    BitTorrentPacket{
        id: CMD_EXTEND_INFO,
        length: 0,
        data: Some(final_payload),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bdecode_extend_info_works() {
        const INPUT: [u8; 519] = [100,50,48,58,97,99,101,95,109,101,116,97,100,97,116,97,95,118,101,114,115,105,111,110,105,49,101,49,51,58,103,101,111,105,112,95,99,111,117,110,116,114,121,50,58,69,83,51,58,108,115,112,105,49,49,53,51,50,56,101,49,58,109,100,49,49,58,117,116,95,109,101,116,97,100,97,116,97,105,50,101,101,50,58,109,105,100,50,48,58,100,105,115,116,97,110,99,101,95,102,114,111,109,95,115,111,117,114,99,101,105,48,101,57,58,100,111,119,110,95,114,97,116,101,105,53,57,50,49,55,49,101,49,57,58,100,111,119,110,108,111,97,100,95,119,105,110,100,111,119,95,101,110,100,105,49,49,53,51,50,56,101,49,51,58,105,115,95,97,99,99,101,115,115,105,98,108,101,105,49,101,49,54,58,108,105,118,101,95,119,105,110,100,111,119,95,115,105,122,101,105,49,49,48,101,51,58,108,115,112,105,49,49,53,51,50,56,101,51,58,109,97,109,105,45,49,101,57,58,109,97,120,95,112,105,101,99,101,105,49,49,53,51,50,55,101,57,58,109,105,110,95,112,105,101,99,101,105,49,49,52,55,51,50,101,57,58,112,101,101,114,95,116,121,112,101,105,48,101,49,54,58,112,105,110,103,95,102,114,111,109,95,115,111,117,114,99,101,105,48,101,56,58,112,111,115,105,116,105,111,110,105,49,49,53,51,49,54,101,49,54,58,116,105,109,101,95,102,114,111,109,95,115,111,117,114,99,101,105,45,49,101,49,57,58,116,111,112,95,115,101,115,115,105,111,110,95,117,112,95,114,97,116,101,105,50,50,54,53,57,48,53,101,49,49,58,116,111,112,95,117,112,95,114,97,116,101,105,50,50,51,55,55,51,54,101,55,58,117,112,95,114,97,116,101,105,52,53,49,55,53,101,49,51,58,117,112,108,111,97,100,95,114,97,116,105,110,103,105,48,101,101,50,58,110,116,105,49,101,49,58,112,105,56,54,50,49,101,56,58,112,108,97,116,102,111,114,109,105,51,101,50,58,112,118,105,50,101,50,58,116,115,105,49,54,51,50,57,49,55,49,52,55,101,50,58,116,116,50,58,98,116,49,58,118,105,51,48,49,54,55,48,49,101,54,58,121,111,117,114,105,112,52,58,87,219,33,178,101];
        let resp = serde_bencode::de::from_bytes::<crate::acestream::bittorrent::InitialExtendInfo>(&INPUT).unwrap();
        println!("Extend info: {:?}", resp);

        let ok = resp.tt.eq("bt") && (resp.p == 8621);
        assert_eq!(ok, true);
    }
}