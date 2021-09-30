
use serde::{Serialize, Deserialize};
use serde_bytes::ByteBuf;

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
    pub pubkey: Option<ByteBuf>,
    pub categories: Vec<String>,
}

impl Torrent {
    pub fn clone(&self) -> Torrent {
        Torrent {
            player_id: self.player_id.clone(),
            infohash: self.infohash.clone(),
            encrypted_sha1: self.encrypted_sha1.clone(),
            name: self.name.clone(),
            piece_length: self.piece_length,
            chunk_length: self.chunk_length,
            bitrate: self.bitrate,
            authmethod: self.authmethod.clone(),
            quality: self.quality.clone(),
            trackers: self.trackers.clone(),
            pubkey: self.pubkey.clone(),
            categories: self.categories.clone(),
        }
    }
}
