use acelib::acestream::{bittorrent, utils};
use acelib::server;

const VERSION: &str = "0.1";
const HELP: &str = "acestream-rs 0.1

USAGE:
    acestream-rs [FLAGS] [OPTIONS]

FLAGS:
    -h, --help       Prints help information
        --http       HTTP as output server
    -V, --version    Prints version information

OPTIONS:
        --dport <dport>    DHT UDP port
    -p <playerid>          Player ID/Content ID
    -P <port>              Output server port
    -t <torrent>           Torrent file
    -w <workers>           Number of workers (threads) to download video";

struct Options {
    torrent: String,
    player_id: String,
    workers: usize,
    port: u16,
    use_http: bool,
    dht_port: Option<u16>,
}

enum Command {
    Run(Options),
    Help,
    Version,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let mut torrent = String::new();
    let mut player_id = String::new();
    let mut workers = None;
    let mut port = None;
    let mut use_http = false;
    let mut dht_port = None;

    while let Some(arg) = args.next() {
        // Accepts "-x value", "-xvalue", "-x=value", "--long value" and "--long=value".
        let (name, inline) = match arg.strip_prefix("--") {
            Some(long) => match long.split_once('=') {
                Some((n, v)) => (format!("--{n}"), Some(v.to_owned())),
                None => (arg.clone(), None),
            },
            None if arg.len() > 2 && arg.starts_with('-') && arg.is_char_boundary(2) => {
                let v = &arg[2..];
                (
                    arg[..2].to_owned(),
                    Some(v.strip_prefix('=').unwrap_or(v).to_owned()),
                )
            }
            None => (arg.clone(), None),
        };
        let mut value = |label: &str| -> Result<String, String> {
            inline.clone().or_else(|| args.next()).ok_or_else(|| {
                format!("The argument '{label}' requires a value but none was supplied")
            })
        };
        match name.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--http" if inline.is_none() => use_http = true,
            "-t" => torrent = value("-t <torrent>")?,
            "-p" => player_id = value("-p <playerid>")?,
            "-w" => workers = Some(parse_number(&value("-w <workers>")?, "workers")?),
            "-P" => port = Some(parse_number(&value("-P <port>")?, "port")?),
            "--dport" => dht_port = Some(parse_number(&value("--dport <dport>")?, "dport")?),
            _ => return Err(format!("Found argument '{arg}' which wasn't expected")),
        }
    }

    Ok(Command::Run(Options {
        torrent,
        player_id,
        workers: workers.unwrap_or(6),
        port: port.unwrap_or(4000),
        use_http,
        dht_port,
    }))
}

fn parse_number<T: std::str::FromStr>(value: &str, name: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("Invalid value '{value}' for <{name}>"))
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

fn main() {
    let opts = match parse_args(std::env::args().skip(1)) {
        Ok(Command::Run(opts)) => opts,
        Ok(Command::Help) => {
            println!("{HELP}");
            return;
        }
        Ok(Command::Version) => {
            println!("acestream-rs {VERSION}");
            return;
        }
        Err(e) => fail(&format!("error: {e}\n\n{HELP}")),
    };

    let torrent = if !opts.player_id.is_empty() {
        utils::torrent_from_player_id(&opts.player_id)
    } else if !opts.torrent.is_empty() {
        utils::torrent_from_encrypted_torrentfile(&opts.torrent)
    } else {
        fail("ERROR: Provide torrent or player ID!");
    };
    let torrent = torrent.unwrap_or_else(|e| fail(&format!("ERROR: {e}")));

    let tx = if opts.use_http {
        // TODO: not working (only VLC..)
        let mut out_server = server::http::OutHttpServer::new();
        out_server.run_forever(opts.port);
        out_server.get_tx()
    } else {
        // start raw TCP server to serve video pieces
        let mut out_server = server::tcp::OutTcpServer::new();
        out_server.run_forever(opts.port);
        out_server.get_tx()
    };

    println!("{torrent:?}");
    let mut bt_client = bittorrent::new_client(torrent, Some(tx));
    bt_client.start(opts.workers, opts.dht_port);
}
