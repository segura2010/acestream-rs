# acestream-rs

A Rust implementation of the AceStream P2P protocol, written by reverse
engineering the original AceStream client.

Only the download side of the protocol is implemented (chunks are never
uploaded to other peers).

## Usage

```
acestream-rs -p <content_id>
```

By default the video is served as a raw TCP stream on port 4000. Open it in
VLC (or ffplay) as `tcp://localhost:4000`.

| Option            | Description                                         | Default |
| ----------------- | --------------------------------------------------- | ------- |
| `-p <playerid>`   | Player ID / content ID                              |         |
| `-t <torrent>`    | Local AceStream torrent file (encrypted or plain)   |         |
| `-w <workers>`    | Number of download workers (threads)                | 6       |
| `-P <port>`       | Output server port                                  | 4000    |
| `--http`          | Serve the stream over HTTP instead of raw TCP       |         |
| `--dport <dport>` | DHT UDP port                                        | 8621    |
| `-h`, `--help`    | Print help                                          |         |
| `-V`, `--version` | Print version                                       |         |

The raw TCP server accepts any number of simultaneous players. The HTTP server
answers one player at a time.

## Building

```
cargo build --release
```

The crate also builds a library (`acelib`) for mobile apps:

* Android: `cargo ndk -t armeabi-v7a -t arm64-v8a -o ./target/jniLibs build --lib --release`
  (JNI entry point `Java_org_videolan_vlc_gui_network_MRLPanelFragment_playacestream`).
* iOS: `cargo lipo --release` (C entry point `playacestream`, see `src/ios/acelib.h`).

## How it works

1. The content ID is exchanged for an AES-128-CBC encrypted torrent through the
   AceStream HTTP API. The torrent checksum is then exchanged for the infohash.
2. Peers are discovered through the BitTorrent mainline DHT (`get_peers`).
3. Each worker connects to a random peer, performs the AceStream handshake,
   learns the live position from the peer's `EXTEND_INFO` message and requests
   pieces chunk by chunk. The signature at the end of every piece is stripped.
4. Completed pieces are forwarded in order to the output server.

## Implementation notes

* No third-party dependencies (except `jni` on Android). HTTP, DNS, bencode,
  SHA-1, AES-128, base64 and the CLI are small purpose-built implementations
  on top of the standard library.
* Peer connections parse packets in place from a reusable receive buffer: each
  video byte is copied once (socket to piece buffer) and the steady state does
  not allocate per packet.
* Piece buffers are recycled through a small pool and handed to the output
  server without copying.
* Pieces are handed to the output through a mutex-protected gate instead of a
  dedicated manager thread; workers read the playback position lock-free.
* The DHT `get_peers` query is serialised once, so sweeping thousands of nodes
  does not allocate. Bootstrap routers are resolved concurrently through
  public DNS servers.

Run the unit tests with `cargo test`.
