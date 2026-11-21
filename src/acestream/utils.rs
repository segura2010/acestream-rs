//! Torrent loading: from a content id (via the API) or from a local
//! (possibly encrypted) torrent file.

use crate::acestream::crypto::{AES_IV, AES_KEY};
use crate::acestream::{api, torrent::Torrent};
use crate::crypto::{aes, sha1::sha1};
use crate::utils::{base64_decode, hex_encode};

/// Length of the "AceStreamTransport" header that precedes the ciphertext.
const TRANSPORT_HEADER_LEN: usize = 20;

/// Parses a plain (not encrypted) torrent and completes it with the API.
pub fn parse_torrent(torrent_bytes: &[u8]) -> Result<Torrent, String> {
    let mut torrent =
        Torrent::from_bencode(torrent_bytes).map_err(|e| format!("parse_torrent_error:{e}"))?;
    fill_from_api(&mut torrent, torrent_bytes)?;
    Ok(torrent)
}

pub fn torrent_from_encrypted_torrent(enc_torrent: &[u8]) -> Result<Torrent, String> {
    let decrypted = enc_torrent
        .get(TRANSPORT_HEADER_LEN..)
        .map(|ciphertext| aes::decrypt_cbc(&AES_KEY, &AES_IV, ciphertext));
    match decrypted.as_deref().map(Torrent::from_bencode) {
        Some(Ok(mut torrent)) => {
            fill_from_api(&mut torrent, enc_torrent)?;
            Ok(torrent)
        }
        // Maybe it is not encrypted: try to parse it as is.
        _ => parse_torrent(enc_torrent).map_err(|e| format!("decryption_error:{e}")),
    }
}

pub fn torrent_from_encrypted_torrentfile(filepath: &str) -> Result<Torrent, String> {
    let data = std::fs::read(filepath).map_err(|e| format!("error_reading_file:{e}"))?;
    torrent_from_encrypted_torrent(&data)
}

pub fn torrent_from_player_id(player_id: &str) -> Result<Torrent, String> {
    let response = api::get_torrent(player_id)?;
    let enc_torrent =
        base64_decode(response.torrent.as_bytes()).ok_or("error_get_torrent_base64decode")?;
    torrent_from_encrypted_torrent(&enc_torrent)
}

/// Fills the identity fields (checksum, player id and infohash).
fn fill_from_api(torrent: &mut Torrent, raw: &[u8]) -> Result<(), String> {
    torrent.encrypted_sha1 = hex_encode(&sha1(raw));
    let check = api::check_torrent(&torrent.encrypted_sha1)?;
    torrent.player_id = check.id;
    torrent.infohash = check.infohash;
    Ok(())
}
