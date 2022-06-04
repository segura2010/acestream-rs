
use crate::acestream::bittorrent;

use std::net::TcpStream;
use std::io::{Write, Read};
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::sync::mpsc::{Sender, Receiver};

pub struct OutTcpServer {
    pub tx: Option<Sender<bittorrent::FullPiece>>,
}

impl OutTcpServer {
    pub fn new() -> Self {
        Self{
            tx: None,
        }
    }

    pub fn get_tx(&self) -> Sender<bittorrent::FullPiece> {
        self.tx.as_ref().unwrap().clone()
    }

    pub fn run_forever(&mut self, port: u16) {
        let (tx_bit, rx_bit) = mpsc::channel();
        self.tx = Some(tx_bit);
        
        let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::with_capacity(10)));

        // thread to consume pieces and forward them to each connected client
        let sender_shared_clients = clients.clone();
        std::thread::spawn(move||{
            loop{
                if let Ok(piece) = rx_bit.recv() {
                    let mut i = 0;
                    let mut clients = sender_shared_clients.lock().unwrap();
                    println!("[S] TCP server clients {}", clients.len());
                    while i < clients.len() {
                        let mut c = &clients[i];
                        if let Err(_) = c.write(&piece.data) {
                            clients.remove(i);
                            i -= 1;
                        }
                        i += 1;
                    }
                }
            }
        });

        // thread to accept new clients and save on shared clients vector
        let shared_clients = clients.clone();
        std::thread::spawn(move||{
            if let Ok(tcp_listener) = std::net::TcpListener::bind(format!("0.0.0.0:{}", port)){
                println!("[S] OutTcpServer listening on port {}..", port);
                for tcp_stream in tcp_listener.incoming() {
                    if let Ok(mut tstream) = tcp_stream {
                        println!("[S] New client connected from {}", tstream.peer_addr().unwrap());
                        tstream.set_write_timeout(
                            Some(std::time::Duration::from_millis(2000))
                        );
                        tstream.set_read_timeout(
                            Some(std::time::Duration::from_millis(1500))
                        );
                        tstream.set_nodelay(true);
                        (shared_clients.lock().unwrap()).push(tstream);
                    }
                }
            } else {
                println!("[S] ERROR: unable to create TcpListener");
            }
        });
    }
}
