
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
        
        let (tx_client, rx_client) = mpsc::channel();
        
        let client_connected: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));

        // just a consumer for when no client is connected
        let shared_client_tx = tx_client.clone();
        let shared_client_connected = client_connected.clone();
        std::thread::spawn(move||{
            loop{
                if let Ok(piece) = rx_bit.recv() {
                    if *(shared_client_connected.lock().unwrap()) {
                        shared_client_tx.send(piece);
                    }
                }
            }
        });

        let shared_client_connected = client_connected.clone();
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
                        *(shared_client_connected.lock().unwrap()) = true;
                        loop{
                            match rx_client.recv() {
                                Ok(piece) => {
                                    if let Err(e) = tstream.write(&piece.data) {
                                        println!("[S] Error sending to client: {}", e);
                                        break;
                                    }
                                },
                                Err(e) => {
                                    println!("[S] Error receiving piece: {}", e);
                                },
                            }
                        }
                        *(shared_client_connected.lock().unwrap()) = false;
                    }
                }
            } else {
                println!("[S] ERROR: unable to create TcpListener");
            }
        });
    }
}
