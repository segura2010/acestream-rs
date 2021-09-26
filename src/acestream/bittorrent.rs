
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::sync::mpsc::{Sender, Receiver};
use std::net::TcpStream;
use std::io::{Write, Read};

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
}
