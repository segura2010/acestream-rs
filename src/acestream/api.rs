//! AceStream HTTP API: content id -> encrypted torrent, and torrent
//! checksum -> infohash.

use crate::crypto::sha1::sha1;
use crate::utils::{base64_decode, hex_encode, http};

const BASE_URL: &str = "http://s1.torrentstream.org";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 6.1) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/60.0.3112.50 Safari/537.36 AceStream/3.1.32";
const VERSION: &str = "3.1.32";
pub const VERSION_CODE: &str = "3013200";
// The escape sequences are intentionally literal (backslash + chars): this
// is the exact salt expected by the API.
const SIGN_SALT: &str = "H!+:H1NnvvX\\x0bS'(;0/A\\nR{${\\n/3%1\\x0b*[r0o>QzNGKkXT@v\\x0b3DN;gx_66L2 {`F0,\\tKm>XoG~iY(\\x0bu]6E}\\t~07&H;9qE1d?d-A7S(";

fn do_get(path: &str, params: &str) -> Result<String, String> {
    let url = format!("{BASE_URL}{path}?{}", sign_request(params));
    let body = http::get(&url, USER_AGENT).map_err(|e| format!("error_http_request:{e}"))?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

fn sign_request(params: &str) -> String {
    let mut transformed = params.replace('&', "#");
    transformed.push_str(SIGN_SALT);
    format!("{params}&_s={}", hex_encode(&sha1(transformed.as_bytes())))
}

#[derive(Debug, PartialEq, Eq)]
pub struct GetTorrentResponse {
    pub torrent: String,
    pub checksum: String,
    pub developer_id: i32,
    pub affiliate_id: i32,
    pub zone_id: i32,
}

pub fn get_torrent(player_id: &str) -> Result<GetTorrentResponse, String> {
    let params =
        format!("_n={VERSION}&_p=win32&_r=128687276&_v={VERSION_CODE}&a=0&pid={player_id}");
    let body = do_get("/gettorrent", &params)?;
    Ok(GetTorrentResponse {
        torrent: required_field(&body, "torrent")?,
        checksum: required_field(&body, "checksum")?,
        developer_id: int_field(&body, "developer_id")?,
        affiliate_id: int_field(&body, "affiliate_id")?,
        zone_id: int_field(&body, "zone_id")?,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct CheckTorrentResponse {
    /// Player (content) id.
    pub id: String,
    /// Hex encoded infohash.
    pub infohash: String,
}

pub fn check_torrent(torrent_hash: &str) -> Result<CheckTorrentResponse, String> {
    let params = format!(
        "_n={VERSION}&_p=win32&_r=128687276&_v={VERSION_CODE}&a=0&checksum={torrent_hash}&d=0&z=0"
    );
    let body = do_get("/checktorrent", &params)?;
    let id = required_field(&body, "id")?;
    let infohash_b64 = required_field(&body, "infohash")?;
    let infohash = base64_decode(infohash_b64.as_bytes()).ok_or("error_infohash_base64decode")?;
    Ok(CheckTorrentResponse {
        id,
        infohash: hex_encode(&infohash),
    })
}

fn required_field(body: &str, name: &str) -> Result<String, String> {
    xml_field(body, name).ok_or_else(|| format!("error_http_body_xml:missing field `{name}`"))
}

fn int_field(body: &str, name: &str) -> Result<i32, String> {
    required_field(body, name)?
        .trim()
        .parse()
        .map_err(|e| format!("error_http_body_xml:field `{name}`: {e}"))
}

/// Returns the (unescaped) text content of the first `<name>` element.
/// The API responses are flat and tiny, so a full XML parser is not needed.
fn xml_field(body: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(i) = body[from..].find('<') {
        let tag_start = from + i + 1;
        let rest = &body[tag_start..];
        from = tag_start;
        let Some(after_name) = rest.strip_prefix(name) else {
            continue;
        };
        // The name must end here (not be a prefix of a longer tag name).
        match after_name.as_bytes().first() {
            Some(b'>' | b'/') => {}
            Some(c) if c.is_ascii_whitespace() => {}
            _ => continue,
        }
        let tag_end = after_name.find('>')?;
        if after_name[..tag_end].ends_with('/') {
            return Some(String::new()); // <name/>
        }
        let content = &after_name[tag_end + 1..];
        let end = content.find(&format!("</{name}"))?;
        return Some(xml_unescape(&content[..end]));
    }
    None
}

fn xml_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let decoded = rest.find(';').and_then(|end| {
            let entity = &rest[1..end];
            let c = match entity {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => entity
                    .strip_prefix("#x")
                    .map(|h| u32::from_str_radix(h, 16))
                    .or_else(|| entity.strip_prefix('#').map(str::parse))
                    .and_then(Result::ok)
                    .and_then(char::from_u32),
            }?;
            Some((c, end + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GET_TORRENT_XML: &str = "<?xml version=\"1.0\"?>\n<response sig=\"1bb2\" r=\"128687276\"><torrent>QWNl</torrent><checksum>2703</checksum><developer_id>0</developer_id><affiliate_id>0</affiliate_id><zone_id>0</zone_id></response>";

    #[test]
    fn extracts_fields() {
        assert_eq!(xml_field(GET_TORRENT_XML, "torrent").unwrap(), "QWNl");
        assert_eq!(xml_field(GET_TORRENT_XML, "zone_id").unwrap(), "0");
        assert!(xml_field(GET_TORRENT_XML, "zone").is_none());
        assert_eq!(xml_field("<a x=\"1\">v</a>", "a").unwrap(), "v");
        assert_eq!(xml_field("<a/>", "a").unwrap(), "");
        assert_eq!(xml_field("<ab>1</ab><a>2</a>", "a").unwrap(), "2");
    }

    #[test]
    fn unescapes_entities() {
        assert_eq!(
            xml_unescape("a&lt;b&amp;c&#65;&#x42;&bogus;"),
            "a<b&cAB&bogus;"
        );
    }

    #[test]
    fn signs_like_the_original_client() {
        // Value computed with the original implementation's algorithm.
        let signed = sign_request(
            "_n=3.1.32&_p=win32&_r=128687276&_v=3013200&a=0&pid=b04372b9543d763bd2dbd2a1842d9723fd080076",
        );
        assert!(
            signed.ends_with("&_s=04143cdba28d7e5f70660a13c69d0d43f630fa24"),
            "{signed}"
        );
    }
}
