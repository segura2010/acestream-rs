# acestream-rs

It is just a Rust re-write of acestream's protocol that comes from reverse engineering the original acestream's protocol.

Only download part of the protocol is implemented (so no upload of chunks to other peers).

Usage: 
``
acestream-rs -p [acestream_streamid]
``

By default a TCP stream will be available at port 4000. You can use it in vlc playing as: tcp://localhost:4000.