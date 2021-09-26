
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::sync::mpsc::{Sender, Receiver};
use std::net::{TcpStream, SocketAddr};
use byteorder::{BigEndian, WriteBytesExt, ReadBytesExt};
use std::io::{Write, Read, Cursor};

use rand::prelude::*;
use rand::seq::SliceRandom;

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
            }
            std::thread::sleep(std::time::Duration::from_millis(1000));
        }
    }

    fn connect(&mut self, peer: dht::Peer) -> Result<u8, String> {
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
        // TODO: receive header and payload
        if let Some(mut socket) = self.socket.as_ref() {
            let mut header = [0u8; 5]; // 4 bytes for length + 1 for message ID
            match socket.read(&mut header) {
                Ok(n) => {
                    let mut packet = packet_from_header(header.to_vec());
                    if packet.length > 50000 || packet.length < 0 {
                        return Err(format!("bad_packet_len:{}", packet.length).to_string());
                    }
                    // TODO: receive packet payload.. we need a loop (?)
                    /*let mut buff = [u8; 10000];
                    match socket.read(&mut buff) {
                        Ok(n) => {
                            packet.id = 
                        },
                        Err(e) => {
                            return Err(e.to_string());
                        },
                    }*/
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
}
