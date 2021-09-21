
use crate::crypto::aes::EncryptorAES;
use crate::acestream::crypto;
use crate::acestream::torrent;
use crate::acestream::api;

use bendy::serde::{from_bytes, to_bytes};

pub fn torrent_from_encrypted_torrent(enc_torrent: &[u8]) -> Result<torrent::Torrent, String> {
    let cipher = EncryptorAES::new_cbc(crypto::AES_KEY, crypto::AES_IV);
    let dec_torrent = cipher.decrypt(enc_torrent.to_vec());
    // println!("{:?}", dec_torrent);
    match from_bytes::<torrent::Torrent>(&dec_torrent){
        Ok(torr) => { return Ok(torr) },
        Err(e) => { return Err(format!("decryption_error:{}", e.to_string())) },
    }
}

pub fn torrent_from_encrypted_torrentfile(filepath: String) -> Result<torrent::Torrent, String> {
    match std::fs::read(filepath){
        Ok(enc_torrent) => {
            let dec_torrent = torrent_from_encrypted_torrent(&enc_torrent[20..]);
            return dec_torrent;
        },
        Err(e) => {
            return Err(format!("error_reading_file:{}", e.to_string()));
        },
    }
}

pub async fn torrent_from_player_id(player_id: String) -> Result<torrent::Torrent, String> {
    let torrent_resp = api::get_torrent(player_id).await;
    match torrent_resp {
        Ok(t) => {
            if let Ok(enc_torrent) = base64::decode(t.torrent) {
                return torrent_from_encrypted_torrent(&enc_torrent[20..]);
            } else {
                return Err("error_get_torrent_base64decode".to_string());
            }
        },
        Err(e) => { return Err(e); },
    }
}
