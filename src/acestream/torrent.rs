use crate::bencode::{self, Value};

/// AceStream torrent metadata.
///
/// `player_id`, `infohash` (hex) and `encrypted_sha1` are not part of the
/// bencoded data: they are filled in from the AceStream API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Torrent {
    pub player_id: String,
    pub infohash: String,
    pub encrypted_sha1: String,

    pub name: Option<String>,
    pub piece_length: usize,
    pub chunk_length: usize,
    pub bitrate: usize,
    pub authmethod: Option<String>,
    pub quality: Option<String>,
    pub trackers: Option<Vec<String>>,
    pub pubkey: Option<Vec<u8>>,
    pub categories: Option<Vec<String>>,
}

impl Torrent {
    /// Parses the bencoded torrent dictionary. Unknown keys are ignored.
    pub fn from_bencode(data: &[u8]) -> Result<Torrent, String> {
        let root = bencode::decode(data).map_err(|e| e.to_string())?;
        if root.dict().is_none() {
            return Err("expected a dictionary".into());
        }
        Ok(Torrent {
            player_id: String::new(),
            infohash: String::new(),
            encrypted_sha1: String::new(),
            name: optional(&root, "name", |v| v.as_str().map(str::to_owned))?,
            piece_length: required(&root, "piece_length", as_usize)?,
            chunk_length: required(&root, "chunk_length", as_usize)?,
            bitrate: required(&root, "bitrate", as_usize)?,
            authmethod: optional(&root, "authmethod", |v| v.as_str().map(str::to_owned))?,
            quality: optional(&root, "quality", |v| v.as_str().map(str::to_owned))?,
            trackers: optional(&root, "trackers", string_list)?,
            pubkey: optional(&root, "pubkey", |v| v.as_bytes().map(<[u8]>::to_vec))?,
            categories: optional(&root, "categories", string_list)?,
        })
    }

    /// Raw 20-byte infohash.
    pub fn infohash_bytes(&self) -> Option<[u8; 20]> {
        crate::utils::hex_decode(&self.infohash)?.try_into().ok()
    }
}

fn as_usize(v: &Value<'_>) -> Option<usize> {
    v.as_int().and_then(|i| usize::try_from(i).ok())
}

fn string_list(v: &Value<'_>) -> Option<Vec<String>> {
    v.list()?
        .map(|item| item.as_str().map(str::to_owned))
        .collect()
}

fn optional<T>(
    root: &Value<'_>,
    key: &str,
    f: impl Fn(&Value<'_>) -> Option<T>,
) -> Result<Option<T>, String> {
    match root.get(key.as_bytes()) {
        None => Ok(None),
        Some(v) => f(&v)
            .map(Some)
            .ok_or_else(|| format!("invalid type for field `{key}`")),
    }
}

fn required<T>(
    root: &Value<'_>,
    key: &str,
    f: impl Fn(&Value<'_>) -> Option<T>,
) -> Result<T, String> {
    optional(root, key, f)?.ok_or_else(|| format!("missing field `{key}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_torrent_dictionary() {
        let data = b"d21:allow_public_trackersi1e10:authmethod3:RSA7:bitratei646899e10:categoriesl8:regionale12:chunk_lengthi16384e4:name17:Sky Sport F1 [DE]12:piece_lengthi524288e6:pubkey3:\x00\x01\x027:quality2:SD8:trackersl3:t:1ee";
        let t = Torrent::from_bencode(data).unwrap();
        assert_eq!(t.name.as_deref(), Some("Sky Sport F1 [DE]"));
        assert_eq!(
            (t.piece_length, t.chunk_length, t.bitrate),
            (524288, 16384, 646899)
        );
        assert_eq!(t.authmethod.as_deref(), Some("RSA"));
        assert_eq!(t.quality.as_deref(), Some("SD"));
        assert_eq!(t.trackers, Some(vec!["t:1".to_owned()]));
        assert_eq!(t.pubkey, Some(vec![0, 1, 2]));
        assert_eq!(t.categories, Some(vec!["regional".to_owned()]));
    }

    #[test]
    fn rejects_invalid_torrents() {
        assert!(Torrent::from_bencode(b"d12:chunk_lengthi16384e7:bitratei1ee").is_err()); // no piece_length
        assert!(
            Torrent::from_bencode(b"d7:bitratei1e12:chunk_lengthi-1e12:piece_lengthi1ee").is_err()
        );
        assert!(Torrent::from_bencode(b"le").is_err());
    }
}
