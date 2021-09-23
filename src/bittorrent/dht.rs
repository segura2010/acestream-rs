
use serde::{Serialize, Deserialize};
use bendy::serde::{from_bytes, to_bytes};
use serde_bytes::ByteBuf;

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
    //pub ro: 
    pub t: i32, // transaction ID
    pub y: String, // message type
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Node { //(Vec<u8>, i64);
    #[serde(with = "serde_bytes")]
    pub id: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub ip: Vec<u8>,
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DHTResponseResponse {
    #[serde(with = "serde_bytes")]
    pub id: Vec<u8>,
    //#[serde(with = "serde_bytes")]
    //pub nodes: Option<Vec<u8>>,
    //#[serde(with = "serde_bytes")]
    //pub values: Option<Vec<u8>>, // peers
    #[serde(with = "serde_bytes")]
    pub token: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DHTResponse {
    pub r: DHTResponseResponse,
    #[serde(with = "serde_bytes")]
    pub v: Vec<u8>, // version
    #[serde(with = "serde_bytes")]
    pub t: Vec<u8>, // transaction ID
    pub y: String, // message type
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

pub struct DHTManager {
    pub routers: Vec<std::net::SocketAddr>,

    pub socket: Option<std::net::UdpSocket>,
    // TODO: save list of nodes?
}

impl DHTManager {
    pub fn listen(&mut self, binding: String) -> Result<u8, String> {
        match std::net::UdpSocket::bind(binding) {
            Ok(s) => {
                self.socket = Some(s);
                return Ok(0);
            }
            Err(e) => {
                return Err(e.to_string());
            }
        }
    }

    // TODO: should return channel to receive an updated list of peers..
    pub fn get_peers(&self, infohash: &[u8]) {
        if let Some(socket) = &self.socket {
            let msg = GetPeersRequest{
                q: "get_peers".to_string(),
                t: 1,
                y: "q".to_string(),
                a: GetPeersRequestTrasaction{
                    id: infohash.to_vec(), // TODO: should be some random 20 bytes slice..
                    info_hash: infohash.to_vec(),
                    want: vec!["n4".to_string(), "n6".to_string()],
                },
            };

            for r in self.routers.iter(){
                socket.send_to(&msg.to_bencode().unwrap(), r);
            }
        }
    }

    pub fn recv_peers(&self) {
        if let Some(socket) = &self.socket {
            let mut buff = [0u8; 1024];
            if let Ok((n, src)) = socket.recv_from(&mut buff) {
                println!("{}, {:?}: {:?}", n, src, buff);
                // TODO: check other bencode/bdecode libs.. this one does not support Option..
                println!("{:?}", from_bytes::<DHTResponse>(&buff[..n]));
            }
        }
    }
}
