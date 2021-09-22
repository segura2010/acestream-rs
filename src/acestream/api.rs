
use sha1::{Sha1, Digest};
use serde_xml_rs::{from_str, to_string};
use serde::{Serialize, Deserialize};

const BASE_URL: &str = "http://s1.torrentstream.org";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 6.1) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/60.0.3112.50 Safari/537.36 AceStream/3.1.32";
const VERSION: &str = "3.1.32";
const VERSION_CODE: &str = "3013200";
const SIGN_SALT: &str = "H!+:H1NnvvX\\x0bS'(;0/A\\nR{${\\n/3%1\\x0b*[r0o>QzNGKkXT@v\\x0b3DN;gx_66L2 {`F0,\\tKm>XoG~iY(\\x0bu]6E}\\t~07&H;9qE1d?d-A7S(";

async fn _do_get(url: String) -> Result<reqwest::Response, String> {
    let res = reqwest::Client::new()
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await;
    match res {
        Ok(response) => { return Ok(response) },
        Err(e) => { return Err(format!("error_http_request:{}", e)) },
    }
}

async fn do_get(path: String, params: String) -> Result<reqwest::Response, String> {
    let signed_params = sign_request(params);
    let url = format!("{}{}?{}", BASE_URL, path, signed_params);
    return _do_get(url).await;
}

fn sign_request(params: String) -> String {
    let params_transform = format!("{}{}", params.replace("&", "#"), SIGN_SALT);
    let mut hasher = Sha1::new();
    hasher.update(params_transform.as_bytes());
    let hash_bytes = hasher.finalize();
    return format!("{}&_s={}", params, hex::encode(hash_bytes));
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct GetTorrentResponse {
    pub torrent: String,
    pub checksum: String,
    pub developer_id: i32,
    pub affiliate_id: i32,
    pub zone_id: i32,
}
pub async fn get_torrent(player_id: String) -> Result<GetTorrentResponse, String> {
    let params = format!("_n={}&_p=win32&_r=128687276&_v={}&a=0&pid={}", VERSION, VERSION_CODE, player_id);
    let response = do_get("/gettorrent".to_string(), params.to_string()).await;
    match response {
        Ok(r) => {
            match r.text().await {
                Ok(body) => {
                    //println!("{}", body);
                    match from_str(&body) {
                        Ok(t) => { return Ok(t) },
                        Err(e) => { return Err(format!("error_http_body_xml:{}", e)) }
                    }
                },
                Err(e) => { return Err(format!("error_http_body:{}", e)) }
            }
        },
        Err(e) => { return Err(e) },
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CheckTorrentResponse {
    pub id: String, // player_id
    pub infohash: String,
}
pub async fn check_torrent(torrent_hash: String) -> Result<CheckTorrentResponse, String> {
    let params = format!("_n={}&_p=win32&_r=128687276&_v={}&a=0&checksum={}&d=0&z=0", VERSION, VERSION_CODE, torrent_hash);
    let response = do_get("/checktorrent".to_string(), params.to_string()).await;
    match response {
        Ok(r) => {
            match r.text().await {
                Ok(body) => {
                    //println!("{}", body);
                    match from_str::<CheckTorrentResponse>(&body) {
                        Ok(mut t) => {
                            t.infohash = hex::encode(base64::decode(t.infohash).unwrap());
                            return Ok(t)
                        },
                        Err(e) => { return Err(format!("error_http_body_xml:{}", e)) }
                    }
                },
                Err(e) => { return Err(format!("error_http_body:{}", e)) }
            }
        },
        Err(e) => { return Err(e) },
    }
}