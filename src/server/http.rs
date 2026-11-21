use std::sync::mpsc::Sender;

use super::output::{self, Mode};
use crate::acestream::bittorrent::FullPiece;

/// Minimal HTTP output server: answers every client with an endless
/// `video/mpeg` response. Serves one client at a time.
#[derive(Default)]
pub struct OutHttpServer {
    tx: Option<Sender<FullPiece>>,
}

impl OutHttpServer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the channel that feeds the server. Panics if the server is not
    /// running.
    pub fn get_tx(&self) -> Sender<FullPiece> {
        self.tx.clone().expect("OutHttpServer is not running")
    }

    /// Starts the server threads (non-blocking).
    pub fn run_forever(&mut self, port: u16) {
        self.tx = Some(output::spawn(port, Mode::Http));
    }
}
