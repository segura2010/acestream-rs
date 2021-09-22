
mod crypto;
mod acestream;

#[tokio::main]
async fn main() {
    println!("TODO: All");
    println!("{:?}", acestream::utils::torrent_from_encrypted_torrentfile("/Users/alberto/Downloads/liga.torrent".to_string()).await);
    println!("{:?}", acestream::utils::torrent_from_player_id("513adf8a4e5352ea4ca183e6faade41e7b04af9a".to_string()).await);
}