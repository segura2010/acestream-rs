
use crate::acestream::torrent;
use crate::bittorrent::dht;

const DHT_PORT: u16 = 8621;
const BITTORRENT_PORT: u16 = 8621;

pub struct BitTorrentClient {
    torrent: torrent::Torrent,
    dht: dht::DHTManager,
}

pub fn new_client(t: torrent::Torrent) -> BitTorrentClient {
    BitTorrentClient{
        torrent: t,
        dht: dht::new_default(),
    }
}

impl BitTorrentClient {
    pub fn start(&mut self) {
        let infohash = hex::decode(self.torrent.infohash.as_str().to_string()).unwrap();
        self.dht.infohash = infohash.as_slice().to_vec();
        self.dht.listen(format!("0.0.0.0:{}", DHT_PORT));

        self.dht.get_peers(&infohash);
        let mut i = 0;
        loop{
            println!("Updating peers..");
            self.dht.recv_peers();
            println!("Nodes: {:?}", self.dht.nodes);
            println!("Peers: {:?}", self.dht.peers);
            if self.dht.peers.len() > 2 {
                // TODO: start video download
            } else if i == self.dht.nodes.len() {
                self.dht.get_peers(&infohash);
                i = 0;
            }
            i += 1;
            std::thread::sleep(std::time::Duration::from_millis(1000));
        }
    }
}