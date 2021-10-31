#[macro_use]
extern crate lazy_static;

mod crypto;
mod acestream;
mod utils;
mod bittorrent;
mod server;

use std::sync::Mutex;

lazy_static! {
    pub static ref already_playing: Mutex<bool> = Mutex::new(false);
}

pub fn play_acestream_with_playerid(player_id: String, port: u16, workers: usize) {
    if *already_playing.lock().unwrap() {
        panic!("Already playing content!");
    }

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
        *already_playing.lock().unwrap() = true;
        println!("{:?}", t);
        let mut bt_client = acestream::bittorrent::new_client(t, Some(tx));
        bt_client.start(workers);
    }
}

// compile with: cargo ndk -t armeabi-v7a -t arm64-v8a -o ./target/jniLibs build --lib --release
// possible targets: armeabi-v7a arm64-v8a x86 x86_64
#[cfg(target_os = "android")]
mod android {
    use jni::JNIEnv;
    use jni::objects::{JClass, JString};
    use jni::sys::{jstring};
    #[no_mangle]
    pub fn Java_org_videolan_vlc_gui_network_MRLPanelFragment_playacestream(env: JNIEnv, _: JClass, player_id: JString) {
        let p_id: String = env.get_string(player_id).expect("invalid pattern string").into();
        std::thread::spawn(move|| {
            crate::play_acestream_with_playerid(p_id, 4000, 5);
        });
    }
}

// compile with: cargo lipo --release
#[cfg(target_os = "ios")]
mod ios {
    use std::os::raw::{c_char};
    use std::ffi::{CString, CStr};
    #[no_mangle]
    pub fn playacestream(player_id: *const c_char) {
        let c_str = unsafe { CStr::from_ptr(player_id) };
        if let Ok(p_id) = c_str.to_str() {
            std::thread::spawn(move|| {
                crate::play_acestream_with_playerid(p_id.to_string(), 4000, 5);
            });
        }
    }
}