
mod crypto;
mod acestream;

fn main() {
    println!("TODO: All");
    println!("{:?}", 
        acestream::utils::torrent_from_encrypted_torrentfile("C:\\Users\\Alberto\\Downloads\\VAMOS.torrent".to_string()));
}
