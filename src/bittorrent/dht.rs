
use crate::utils;

use std::net::{ToSocketAddrs, SocketAddr, IpAddr};
use std::collections::hash_set::HashSet;

use serde::{Serialize, Deserialize};
use serde_bencode::de::from_bytes;
use serde_bencode::ser::to_bytes;
use serde_bytes::ByteBuf;

use std::io::{Write, Read, Cursor};
use byteorder::{BigEndian, WriteBytesExt, ReadBytesExt};

use rand::prelude::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct GetPeersRequestTrasaction {
    #[serde(with = "serde_bytes")]
    pub id: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub info_hash: Vec<u8>,
    pub want: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetPeersRequest {
    pub a: GetPeersRequestTrasaction, // information about the query
    pub q: String, // query type
    pub ro: i32, // read only
    pub t: i32, // transaction ID
    pub y: String, // message type
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Node {
    pub id: [u8; 20],
    pub ip: [u8; 4],
    pub port: u16,
}
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct Peer {
    pub ip: [u8; 4],
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DHTResponseResponse {
    pub id: Option<ByteBuf>,
    pub nodes: Option<ByteBuf>,
    pub values: Option<Vec<ByteBuf>>, // peers
    pub token: Option<ByteBuf>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DHTResponse {
    pub r: DHTResponseResponse,
    pub v: Option<ByteBuf>, // version
    pub t: Option<ByteBuf>, // transaction ID
    pub y: Option<String>, // message type
}
impl DHTResponse {
    pub fn get_nodes(&self) -> Vec<Node> {
        if let Some(nodes) = &self.r.nodes {
            let nodes_size: usize = nodes.len() / 26;
            let mut result = Vec::with_capacity(nodes_size);
            let mut nodes_buff = Cursor::new(nodes);
            for _ in 0..nodes_size {
                let mut id = [0u8; 20];
                let mut ip = [0u8; 4];
                nodes_buff.read(&mut id).unwrap();
                nodes_buff.read(&mut ip).unwrap();
                let n = Node{
                    id: id,
                    ip: ip,
                    port: nodes_buff.read_u16::<BigEndian>().unwrap(),
                };
                result.push(n);
            }

            return result;
        }

        vec![]
    }
    pub fn get_peers(&self) -> Vec<Peer> {
        if let Some(peers) = &self.r.values {
            let mut result: Vec<Peer> = Vec::with_capacity(peers.len());
            for p in peers.iter() {
                let mut peer_buff = Cursor::new(p);
                let mut ip = [0u8; 4];
                peer_buff.read(&mut ip).unwrap();
                let p = Peer{
                    ip: ip,
                    port: peer_buff.read_u16::<BigEndian>().unwrap(),
                };
                result.push(p);
            }

            return result;
        }

        vec![]
    }
}

impl GetPeersRequest {
    pub fn to_bencode(&self) -> Result<Vec<u8>, String> {
        let encoded = to_bytes(self);
        if let Err(e) = encoded {
            return Err(format!("error_bencoding:{}", e).to_string())
        }
        return Ok(encoded.unwrap());
    }
}

fn new_getpeersrequest(tid: i32, node_id: Vec<u8>, infohash: Vec<u8>) -> GetPeersRequest {
    GetPeersRequest{
        q: "get_peers".to_string(),
        t: tid,
        ro: 1,
        y: "q".to_string(),
        a: GetPeersRequestTrasaction{
            id: node_id.clone(),
            info_hash: infohash.clone(),
            want: vec!["n4".to_string()] // nodes IPv6, "n6".to_string()], 
        },
    }
}

#[derive(Debug)]
pub struct DHTManager {
    pub transaction_id: i32,
    pub node_id: Vec<u8>,
    pub infohash: Vec<u8>,
    pub routers: Vec<std::net::SocketAddr>,
    pub nodes: HashSet<Node>,
    pub peers: HashSet<Peer>,

    pub socket: Option<std::net::UdpSocket>,
}

fn generate_random_id(size: usize) -> Vec<u8> {
    let mut rng = rand::thread_rng();
    let mut result: Vec<u8> = Vec::with_capacity(size);
    for i in 0..size {
        result.push(rng.gen());
    }
    result
}

pub enum DHTServer<'a> {
    DomainNameServer(&'a str, u16),
    IPServer(&'a str, u16)
}

const TRACKERS_LIST: [DHTServer; 10] = [
    DHTServer::DomainNameServer("tracker.opentrackr.org", 1337),
    DHTServer::DomainNameServer("9.rarbg.com", 2810),
    DHTServer::DomainNameServer("tracker.openbittorrent.com", 6969),
    DHTServer::DomainNameServer("router.bittorrent.com", 6881),
    DHTServer::DomainNameServer("router.utorrent.com", 6881),
    DHTServer::DomainNameServer("tracker.coppersurfer.tk", 6969),
    DHTServer::DomainNameServer("dht.transmissionbt.com", 6881),
    DHTServer::DomainNameServer("dht.aelitis.com", 6881),
    DHTServer::DomainNameServer("router.silotis.us", 6881),
    DHTServer::DomainNameServer("dht.libtorrent.org", 25401),
];

pub fn new_default() -> DHTManager {
    let mut routers = Vec::with_capacity(20);
    for tracker in TRACKERS_LIST {
        if let Ok(mut server) = utils::create_dht_router_addr(tracker) {
            routers.append(&mut server);
        }
    }

    DHTManager{
        transaction_id: 0,
        node_id: generate_random_id(20),
        infohash: vec![],
        routers: routers,
        nodes: HashSet::<Node>::with_capacity(100),
        peers: HashSet::<Peer>::with_capacity(200),
        socket: None,
    }
}

impl DHTManager {
    pub fn listen(&mut self, binding: String) -> Result<u8, String> {
        match std::net::UdpSocket::bind(binding) {
            Ok(s) => {
                s.set_read_timeout(Some(std::time::Duration::from_millis(300)));
                s.set_write_timeout(Some(std::time::Duration::from_millis(1000)));
                self.socket = Some(s);
                return Ok(0);
            }
            Err(e) => {
                return Err(e.to_string());
            }
        }
    }

    // TODO: should return channel to receive an updated list of peers.. ?
    pub fn get_peers(&mut self, infohash: &[u8]) {
        if let Some(socket) = &self.socket {
            for r in self.routers.iter(){
                let msg = new_getpeersrequest(self.transaction_id, self.node_id.clone(), infohash.to_vec());
                self.transaction_id += 1;
                socket.send_to(&msg.to_bencode().unwrap(), r);
            }
            for n in self.nodes.iter(){
                let msg = new_getpeersrequest(self.transaction_id, self.node_id.clone(), infohash.to_vec());
                self.transaction_id += 1;
                let addr = SocketAddr::from((n.ip, n.port));
                socket.send_to(&msg.to_bencode().unwrap(), addr);
            }
        }
    }

    pub fn recv_peers(&mut self) -> Result<(), String> {
        if let Some(socket) = &self.socket {
            let mut buff = [0u8; 2048];
            if let Ok((n, src)) = socket.recv_from(&mut buff) {
                //println!("{}, {:?}: {:?}", n, src, buff);
                match from_bytes::<DHTResponse>(&buff[..n]) {
                    Ok(resp) => {
                        //println!("{:?}", resp);
                        for n in resp.get_nodes() {
                            let ip = n.ip;
                            let port = n.port;
                            let inserted = self.nodes.insert(n);
                            if inserted {
                                // query new nodes :)
                                let msg = new_getpeersrequest(self.transaction_id, self.node_id.clone(), self.infohash.clone());
                                self.transaction_id += 1;
                                socket.send_to(&msg.to_bencode().unwrap(), SocketAddr::from((ip, port)));
                            }
                        }
                        for p in resp.get_peers() {
                            self.peers.insert(p);
                        }
                        //println!("{:?}", self);
                        return Ok(());
                    },
                    Err(e) => {
                        //println!("ERROR: {}", e);
                        return Err(e.to_string());
                    },
                }
            }
        }
        Err("nothing received".to_string())
    }

    pub fn get_peers_forever(&mut self, infohash: &[u8]) {
        loop{
            let recv_res = self.recv_peers();
            println!("{:?}", recv_res);
            if let Err(e) = recv_res {
                //self.get_peers(&infohash);
                std::thread::sleep(std::time::Duration::from_millis(1000));
            } else {
                std::thread::sleep(std::time::Duration::from_millis(5000));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bdecode_works() {
        const INPUT: [u8; 356] = [100,49,58,114,100,50,58,105,100,50,48,58,60,56,25,141,42,131,140,155,130,154,204,134,244,101,138,80,149,76,178,239,50,58,105,112,52,58,87,219,33,178,53,58,110,111,100,101,115,50,48,56,58,60,56,24,135,184,217,159,50,227,121,127,232,134,234,230,182,87,164,197,204,132,154,119,10,250,203,60,56,23,214,174,82,144,73,241,241,187,233,235,179,166,219,60,135,12,225,120,50,67,75,172,96,60,56,23,214,174,82,144,73,241,241,187,233,235,179,166,219,60,135,12,225,80,2,3,75,136,188,60,56,22,214,174,82,144,73,241,241,187,233,235,179,166,219,60,135,12,225,154,68,5,156,254,122,60,56,9,43,3,30,77,129,75,232,249,57,145,33,147,180,112,197,64,210,92,57,190,197,26,233,60,56,11,73,241,241,187,233,235,179,166,219,60,135,12,62,153,36,94,82,190,47,120,18,95,101,60,56,13,153,90,252,17,185,200,191,213,35,70,176,175,69,89,37,231,13,37,23,221,159,200,213,60,56,12,19,33,244,111,119,57,178,63,103,254,105,52,171,176,231,54,45,106,102,11,22,182,173,53,58,116,111,107,101,110,50,48,58,82,215,117,175,125,94,63,180,64,184,16,253,246,107,39,143,99,198,60,78,54,58,118,97,108,117,101,115,108,54,58,79,130,127,130,33,174,54,58,151,41,225,24,33,177,54,58,109,66,33,194,33,173,54,58,188,163,51,130,33,173,101,101,49,58,116,49,58,13,49,58,118,52,58,85,84,98,22,49,58,121,49,58,114,101];
        let resp = serde_bencode::de::from_bytes::<crate::bittorrent::dht::DHTResponse>(&INPUT).unwrap();
        println!("{:?}", resp);
        let nodes = resp.get_nodes();
        println!("{:?}", nodes);
        let peers = resp.get_peers();
        println!("{:?}", peers);

        let ok = (nodes.len() == 8) && (peers.len() == 4);
        assert_eq!(ok, true);
    }
}
