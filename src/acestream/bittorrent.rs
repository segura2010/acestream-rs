
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::sync::mpsc::{Sender, Receiver};
use std::net::{TcpStream, SocketAddr};
use byteorder::{BigEndian, WriteBytesExt, ReadBytesExt};
use std::io::{Write, Read, Cursor};

use rand::prelude::*;
use rand::seq::SliceRandom;
use rand::distributions::Slice;

use crate::acestream::torrent;
use crate::bittorrent::dht;

const DHT_PORT: u16 = 8621;
const BITTORRENT_PORT: u16 = 8621;

pub struct BitTorrentClient {
    torrent: torrent::Torrent,
    dht: dht::DHTManager,
    peers: Arc<Mutex<Vec<dht::Peer>>>,
    started_workers: bool,
    
    tx: Sender<Piece>,
    rx: Receiver<Piece>,
}

pub fn new_client(t: torrent::Torrent) -> BitTorrentClient {
    let (tx, rx) = mpsc::channel();
    BitTorrentClient{
        torrent: t,
        dht: dht::new_default(),
        peers: Arc::new(Mutex::new(Vec::with_capacity(30))),
        started_workers: false,
        tx: tx,
        rx: rx,
    }
}

impl BitTorrentClient {
    pub fn start(&mut self, workers: usize) {
        let infohash = hex::decode(self.torrent.infohash.as_str().to_string()).unwrap();
        self.dht.infohash = infohash.as_slice().to_vec();
        self.dht.listen(format!("0.0.0.0:{}", DHT_PORT));

        self.dht.get_peers(&infohash);
        let mut old_peers_len = self.dht.peers.len();
        let mut i = 0;
        loop{
            println!("Updating peers..");
            self.dht.recv_peers();
            println!("Peers: {:?} (len={})", self.dht.peers, self.dht.peers.len());
            println!("Nodes: {:?}", self.dht.nodes.len());
            if !self.started_workers && self.dht.peers.len() > 2 {
                self.start_workers(workers);
                // TODO: create new thread to receive pieces from workers
            } else if i == self.dht.nodes.len() {
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
            i += 1;
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }

    fn start_workers(&mut self, workers: usize) {
        for i in 0..workers {
            let shared_tx = self.tx.clone();
            let shared_peers = self.peers.clone();
            let torrent = self.torrent.clone();

            std::thread::spawn(move||{
                let mut connector = new_connector(torrent, shared_peers, shared_tx);
                connector.start();
            });
        }
    }
}

pub struct BitTorrentPacket {
    length: i32,
    id: i8,
    data: Option<Vec<u8>>,
}
fn packet_from_header(header: Vec<u8>) -> BitTorrentPacket {
    let mut cursor = Cursor::new(header);
    BitTorrentPacket{
        length: cursor.read_i32::<BigEndian>().unwrap(),
        id: cursor.read_i8().unwrap(),
        data: None,
    }
}
impl BitTorrentPacket {
    fn as_bytes(&self) -> Vec<u8> {
        let mut packet = Vec::with_capacity(self.length as usize);
        packet.write_i32::<BigEndian>(self.length);
        packet.write_i8(self.id);
        if let Some(data) = &self.data {
            packet.write(&data);
        }
        packet
    }
}

pub struct Piece {
    unknown1: i32,
    piece_idx: i32,
    unknown2: i64,
    chunk_idx: i16,
    data: Vec<u8>,
}
// TODO: fn new piece from bytes

pub struct PeerConnector {
    torrent: torrent::Torrent,
    peers: Arc<Mutex<Vec<dht::Peer>>>,
    send_chan: Sender<Piece>,
    actual_peer: Option<dht::Peer>,

    socket: Option<TcpStream>,
}

pub fn new_connector(t: torrent::Torrent, peers: Arc<Mutex<Vec<dht::Peer>>>, chan: Sender<Piece>) -> PeerConnector {
    PeerConnector{
        torrent: t,
        peers: peers,
        send_chan: chan,
        actual_peer: None,
        socket: None,
    }
}

impl PeerConnector {
    pub fn start(&mut self) {
        let mut rng = rand::thread_rng();
        loop{
            // TODO: choose random peer, connect, start download
            let peers: Vec<dht::Peer>;
            {
                peers = self.peers.lock().unwrap().clone();
            }
            if let Some(p) = peers.choose(&mut rng) {
                self.actual_peer = Some(*p);
                println!("Worker selected peer: {:?}", self.actual_peer);
                if let Ok(_) = self.connect(self.actual_peer.unwrap()) {
                    self.start_download();
                } else {
                    // TODO: maybe flag?
                    continue;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1000));
        }
    }

    fn start_download(&self) -> Result<usize, String> {
        // TODO: send&recv handshake, send&recv peerid, recv extend_info packet (update actual position),
        // send our extend_info packet, start piece donwload loop
        Err("unimplemented".to_string())
    }

    fn connect(&mut self, peer: dht::Peer) -> Result<usize, String> {
        let conn = TcpStream::connect_timeout(&SocketAddr::from((peer.ip, peer.port)), 
                    std::time::Duration::from_millis(2500));
        match conn {
            Ok(s) => {
                s.set_read_timeout(Some(std::time::Duration::from_millis(5000)));
                s.set_write_timeout(Some(std::time::Duration::from_millis(2000)));
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
            match socket.read(&mut header) {
                Ok(n) => {
                    let mut packet = packet_from_header(header.to_vec());
                    if packet.length > 50000 || packet.length < 0 {
                        return Err(format!("bad_packet_len:{}", packet.length).to_string());
                    }
                    let mut buff = [0u8; 5000];
                    let mut payload = Vec::with_capacity(packet.length as _);
                    let mut read_bytes = 0usize;
                    loop {
                        if read_bytes >= (packet.length as _){
                            break;
                        }
                        match socket.read(&mut buff) {
                            Ok(n) => {
                                read_bytes += n;
                                payload.extend_from_slice(&buff[0..n]);
                                continue;
                            },
                            Err(e) => {
                                return Err(e.to_string());
                            },
                        }
                    }
                    packet.data = Some(payload);
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
            payload.write(&hex::decode(self.torrent.infohash.as_str().to_string()).unwrap());

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
}

fn generate_peer_id(size: usize) -> String {
    let alphabet = Slice::new(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789").unwrap();
    let rng = rand::thread_rng();
    let peer_id: Vec<u8> = rng.sample_iter(&alphabet).take(size).map(|b| { *b }).collect();
    String::from_utf8(peer_id).unwrap()
}