
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Torrent {
    #[serde(skip)]
    pub player_id: String,
    #[serde(skip)]
    pub infohash: String,
    #[serde(skip)]
    pub encrypted_sha1: String,

    pub name: String,
    pub piece_length: usize,
    pub chunk_length: usize,
    pub bitrate: usize,
    pub authmethod: String,
    pub quality: String,
    pub trackers: Vec<String>,
    //pub pubkey: Vec<u8>,
    pub categories: Vec<String>,
}
