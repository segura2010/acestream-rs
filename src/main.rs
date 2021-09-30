
mod crypto;
mod acestream;
mod utils;
mod bittorrent;
mod server;

use clap::{App, Arg};

fn main() {
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
                .arg(clap::Arg::with_name("workers")
                    .short("w")
                    .help("Number of workers (threads) to download video")
                    .required(false)
                    .takes_value(true))
                .arg(clap::Arg::with_name("port")
                    .short("P")
                    .help("Output server port")
                    .required(false)
                    .takes_value(true))
                .get_matches();
    
    let torrent_file = args.value_of("torrent").unwrap_or("");
    let player_id = args.value_of("playerid").unwrap_or("");
    let workers: usize = args.value_of("workers").unwrap_or("6").parse().unwrap();
    let port: u16 = args.value_of("port").unwrap_or("4000").parse().unwrap();

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
    let mut out_server = server::tcp::new_tcpserver();
    out_server.run_forever(port);
    let tx = out_server.get_tx();

    // everything OK
    if let Some(t) = torrent {
        println!("{:?}", t);
        let mut bt_client = acestream::bittorrent::new_client(t, Some(tx));
        bt_client.start(workers);
    }
}