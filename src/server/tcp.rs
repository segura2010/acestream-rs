use std::sync::mpsc::Sender;

use super::output::{self, Mode};
use crate::acestream::bittorrent::FullPiece;

/// Raw TCP output server (e.g. `tcp://localhost:4000` in VLC). Any number of
/// players can be connected at the same time.
#[derive(Default)]
pub struct OutTcpServer {
    tx: Option<Sender<FullPiece>>,
}

impl OutTcpServer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the channel that feeds the server. Panics if the server is not
    /// running.
    pub fn get_tx(&self) -> Sender<FullPiece> {
        self.tx.clone().expect("OutTcpServer is not running")
    }

    /// Starts the server threads (non-blocking).
    pub fn run_forever(&mut self, port: u16) {
        self.tx = Some(output::spawn(port, Mode::Tcp));
    }
}
