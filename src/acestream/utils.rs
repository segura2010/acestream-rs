
use crate::crypto::aes::EncryptorAES;
use crate::acestream::crypto;
use crate::acestream::torrent;

use bendy::serde::{from_bytes, to_bytes};

pub fn torrent_from_encrypted_torrent(enc_torrent: &[u8]) -> Result<torrent::Torrent, String> {
    let cipher = EncryptorAES::new_cbc(crypto::AES_KEY, crypto::AES_IV);
    let dec_torrent = cipher.decrypt(enc_torrent.to_vec());
    // println!("{:?}", dec_torrent);
    match from_bytes::<torrent::Torrent>(&dec_torrent){
        Ok(torr) => { return Ok(torr) },
        Err(e) => { return Err(e.to_string()) },
    }
}

pub fn torrent_from_encrypted_torrentfile(filepath: String) -> Result<torrent::Torrent, String> {
    let enc_torrent = std::fs::read(filepath).unwrap();
    let dec_torrent = torrent_from_encrypted_torrent(&enc_torrent[20..]);
    return dec_torrent;
}

pub fn torrent_from_player_id() -> Result<torrent::Torrent, String> {
    // TODO: do request to API, get encrypted torrent, decrypt and parse it
    return Err("Unimplemented".to_string());
}
