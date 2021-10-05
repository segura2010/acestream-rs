mod crypto;
mod acestream;
mod utils;
mod bittorrent;
mod server;

pub fn play_acestream_with_playerid(player_id: String, port: String, workers: usize) -> bool {
    let mut torrent: Option<acestream::torrent::Torrent> = None;
    if player_id.len() > 0 {
        match acestream::utils::torrent_from_player_id(player_id.to_string()) {
            Ok(t) => {
                torrent = Some(t);
            },
            Err(e) => {
                panic!(e)
            }
        }
    } else if torrent_file.len() > 0 {
        match acestream::utils::torrent_from_encrypted_torrentfile(torrent_file.to_string()) {
            Ok(t) => {
                torrent = Some(t);
            },
            Err(e) => {
                panic!(e)
            }
        }
    } else {
        panic!("ERROR: Provide torrent or player ID!");
    }

    // start raw TCP server to serve video pieces
    let mut out_server = server::tcp::OutTcpServer::new();
    out_server.run_forever(port);
    tx = out_server.get_tx();

    if let Some(t) = torrent {
        println!("{:?}", t);
        let mut bt_client = acestream::bittorrent::new_client(t, Some(tx));
        bt_client.start(workers);
    }
}

// compile with: cargo ndk -t armeabi-v7a -t arm64-v8a build --lib --release
// for OpenSSL cross-compilation: https://docs.rs/crate/openssl-sys/0.9.19
#[cfg(target_os = "android")]
#[no_mangle]
pub fn play_acestream(id: String) -> bool {
    lib::play_acestream_with_playerid(id, 4000, 5)
}