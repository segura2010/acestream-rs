//! Download-only client for the AceStream P2P protocol.

pub mod acestream;
pub mod bittorrent;
pub mod server;

mod bencode;
mod crypto;
mod utils;

use std::sync::atomic::{AtomicBool, Ordering};

static ALREADY_PLAYING: AtomicBool = AtomicBool::new(false);

/// Fetches the content, serves it on a raw TCP server at `port` and blocks
/// forever downloading it. Only one content can be played per process.
pub fn play_acestream_with_playerid(player_id: String, port: u16, workers: usize) {
    if player_id.is_empty() {
        eprintln!("ERROR: Provide torrent or player ID!");
        return;
    }
    if ALREADY_PLAYING.swap(true, Ordering::SeqCst) {
        eprintln!("Already playing content!");
        return;
    }

    let torrent = match acestream::utils::torrent_from_player_id(&player_id) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("ERROR: {e}");
            ALREADY_PLAYING.store(false, Ordering::SeqCst);
            return;
        }
    };

    // start raw TCP server to serve video pieces
    let mut out_server = server::tcp::OutTcpServer::new();
    out_server.run_forever(port);

    println!("{torrent:?}");
    let mut bt_client = acestream::bittorrent::new_client(torrent, Some(out_server.get_tx()));
    bt_client.start(workers, None);
}

// compile with: cargo ndk -t armeabi-v7a -t arm64-v8a -o ./target/jniLibs build --lib --release
// possible targets: armeabi-v7a arm64-v8a x86 x86_64
#[cfg(target_os = "android")]
mod android {
    use jni::JNIEnv;
    use jni::objects::{JClass, JString};

    #[unsafe(no_mangle)]
    #[allow(non_snake_case)]
    pub extern "system" fn Java_org_videolan_vlc_gui_network_MRLPanelFragment_playacestream(
        env: JNIEnv,
        _: JClass,
        player_id: JString,
    ) {
        let p_id: String = match env.get_string(player_id) {
            Ok(s) => s.into(),
            Err(_) => {
                eprintln!("ERROR: invalid player id string");
                return;
            }
        };
        std::thread::spawn(move || {
            crate::play_acestream_with_playerid(p_id, 4000, 5);
        });
    }
}

// compile with: cargo lipo --release
#[cfg(target_os = "ios")]
mod ios {
    use std::ffi::CStr;
    use std::os::raw::c_char;

    /// # Safety
    /// `player_id` must be null or point to a valid NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn playacestream(player_id: *const c_char) {
        if player_id.is_null() {
            return;
        }
        let c_str = unsafe { CStr::from_ptr(player_id) };
        if let Ok(p_id) = c_str.to_str() {
            let p_id = p_id.to_owned();
            std::thread::spawn(move || {
                crate::play_acestream_with_playerid(p_id, 4000, 5);
            });
        }
    }
}
