
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
        let mut infohash = hex::decode(self.torrent.infohash.as_str().to_string()).unwrap();
        self.dht.infohash = infohash.as_slice().to_vec();
        self.dht.listen(format!("0.0.0.0:{}", DHT_PORT));

        // TODO: check what is happening.. I receive nodes but not peers..
        self.dht.get_peers(&infohash);
        loop{
            println!("Updating peers..");
            self.dht.recv_peers();
            println!("Nodes: {:?}", self.dht.nodes);
            println!("Peers: {:?}", self.dht.peers);
            if self.dht.peers.len() > 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1000));
        }

        loop{
            std::thread::sleep(std::time::Duration::from_millis(1000));
        }
    }
}