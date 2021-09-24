
mod crypto;
mod acestream;
mod utils;
mod bittorrent;

use clap::{App, Arg};

#[tokio::main]
async fn main() {
    let args = clap::App::new("acestream-rs")
                .version("0.1")
                .arg(clap::Arg::with_name("torrent")
                    .short("t")
                    .help("Torrent file")
                    .required(false)
                    .takes_value(true))
                .arg(clap::Arg::with_name("playerid")
                    .short("p")
                    .help("Player ID/Content ID")
                    .required(false)
                    .takes_value(true))
                .get_matches();
    
    let torrent_file = args.value_of("torrent").unwrap_or("");
    let player_id = args.value_of("playerid").unwrap_or("");

    let mut torrent: Option<acestream::torrent::Torrent> = None;
    if player_id.len() > 0 {
        match acestream::utils::torrent_from_player_id(player_id.to_string()).await {
            Ok(t) => {
                torrent = Some(t);
            },
            Err(e) => {
                panic!(e)
            }
        }
    } else if torrent_file.len() > 0 {
        match acestream::utils::torrent_from_encrypted_torrentfile(torrent_file.to_string()).await {
            Ok(t) => {
                torrent = Some(t);
            },
            Err(e) => {
                panic!(e)
            }
        }
    } else {
        // TODO: remove code here; used for testing..

        let mut dht_manager = bittorrent::dht::new_default();
        dht_manager.listen("0.0.0.0:1337".to_string());
        let ihash = hex::decode("4fcc50395f0f4a26af71d82fe8401559c6a4871f").unwrap();
        dht_manager.get_peers(&ihash);
        loop {
            println!("receiving..");
            dht_manager.recv_peers();
            std::thread::sleep(std::time::Duration::from_millis(1000));
            dht_manager.get_peers(&ihash);
        }

        panic!("ERROR: Provide torrent or player ID!");
    }

    // everything OK
    if let Some(t) = torrent {
        println!("{:#?}", t);
        // TODO: look for peers, downlaod, play :)
        // the following code must be moved to a class; it is just a test
        // communication with trackers, some example code: https://github.com/mandreyel/cratetorrent/blob/master/cratetorrent/src/tracker.rs
        if let Ok(router) = utils::create_dht_router_addr("router.bittorrent.com", 6881, true) {
            println!("{:?}", router);
        }
    }
}