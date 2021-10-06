mod crypto;
mod acestream;
mod utils;
mod bittorrent;
mod server;

use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::{jstring};

pub fn play_acestream_with_playerid(player_id: String, port: u16, workers: usize) {
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
    } else {
        panic!("ERROR: Provide torrent or player ID!");
    }

    // start raw TCP server to serve video pieces
    let mut out_server = server::tcp::OutTcpServer::new();
    out_server.run_forever(port);
    let tx = out_server.get_tx();

    if let Some(t) = torrent {
        println!("{:?}", t);
        let mut bt_client = acestream::bittorrent::new_client(t, Some(tx));
        bt_client.start(workers);
    }
}

// compile with: cargo ndk -t armeabi-v7a -t arm64-v8a build --lib --release
// possible targets: armeabi-v7a arm64-v8a x86 x86_64
#[cfg(target_os = "android")]
#[no_mangle]
pub fn Java_org_videolan_vlc_gui_network_playacestream(env: JNIEnv, _: JClass, player_id: JString) {
    let p_id: String = env.get_string(player_id).expect("invalid pattern string").into();
    play_acestream_with_playerid(p_id, 4000, 5)
}