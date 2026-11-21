//! Shared implementation of the TCP and HTTP output servers.
//!
//! Two threads: one accepts players, the other writes every received piece
//! to the connected players. Pieces are not copied: the buffer produced by
//! the download workers is written as is and then returned to its pool.

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use crate::acestream::bittorrent::FullPiece;

const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_millis(2000);
const HTTP_HEADERS: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Type: video/mpeg\r\nAccept-Ranges: bytes\r\n\r\n";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Raw stream to any number of simultaneous clients.
    Tcp,
    /// HTTP response to a single client at a time.
    Http,
}

#[derive(Default)]
struct Clients {
    list: Mutex<Vec<TcpStream>>,
    /// Signalled when the last client is gone (used by the HTTP mode).
    empty: Condvar,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Starts the server threads and returns the channel that feeds them.
pub(crate) fn spawn(port: u16, mode: Mode) -> Sender<FullPiece> {
    let (tx, rx) = mpsc::channel();
    let clients = Arc::new(Clients::default());

    let sender_clients = clients.clone();
    if let Err(e) = thread::Builder::new()
        .name("out-sender".into())
        .spawn(move || send_loop(rx, &sender_clients, mode))
    {
        println!("[S] ERROR: unable to start sender thread: {e}");
    }

    match TcpListener::bind(("0.0.0.0", port)) {
        Ok(listener) => {
            let name = if mode == Mode::Tcp {
                "OutTcpServer"
            } else {
                "OutHttpServer"
            };
            println!("[S] {name} listening on port {port}..");
            let spawned = thread::Builder::new()
                .name("out-accept".into())
                .spawn(move || accept_loop(&listener, &clients, mode));
            if let Err(e) = spawned {
                println!("[S] ERROR: unable to start accept thread: {e}");
            }
        }
        Err(_) => println!("[S] ERROR: unable to create TcpListener"),
    }
    tx
}

fn accept_loop(listener: &TcpListener, clients: &Clients, mode: Mode) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        if let Ok(addr) = stream.peer_addr() {
            println!("[S] New client connected from {addr}");
        }
        let _ = stream.set_write_timeout(Some(CLIENT_WRITE_TIMEOUT));
        let _ = stream.set_nodelay(true);
        match mode {
            Mode::Tcp => lock(&clients.list).push(stream),
            Mode::Http => {
                if stream.write_all(HTTP_HEADERS).is_err() {
                    continue;
                }
                let mut list = lock(&clients.list);
                list.push(stream);
                // One client at a time: wait until it disconnects.
                while !list.is_empty() {
                    list = clients.empty.wait(list).unwrap_or_else(|e| e.into_inner());
                }
            }
        }
    }
}

fn send_loop(rx: Receiver<FullPiece>, clients: &Clients, mode: Mode) {
    // Pieces received while no client is connected are simply dropped.
    for piece in rx {
        let mut list = lock(&clients.list);
        if mode == Mode::Tcp {
            println!("[S] TCP server clients {}", list.len());
        }
        list.retain(|client| match (&mut &*client).write_all(&piece.data) {
            Ok(()) => true,
            Err(e) => {
                if mode == Mode::Http {
                    println!("[S] Error sending to client: {e}");
                }
                false
            }
        });
        if list.is_empty() {
            clients.empty.notify_all();
        }
    }
}
