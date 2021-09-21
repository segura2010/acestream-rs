
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Torrent {
    #[serde(skip)]
    player_id: String,
    #[serde(skip)]
    infohash: String,
    #[serde(skip)]
    encrypted_sha1: String,

    name: String,
    piece_length: usize,
    chunk_length: usize,
    bitrate: usize,
    authmethod: String,
    quality: String,
    trackers: Vec<String>,
    //pubkey: Vec<u8>,
    categories: Vec<String>,
}
