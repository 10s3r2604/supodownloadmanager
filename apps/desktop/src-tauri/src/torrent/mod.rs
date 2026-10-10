//! BitTorrent downloads from magnet links and `.torrent` files, powered by
//! [librqbit](https://github.com/ikatson/rqbit).

mod engine;
#[cfg(test)]
pub(crate) mod testing;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use engine::{EngineConfig, JobConfig, TorrentControl, TorrentEngine, TorrentLive};

/// The largest `.torrent` file SDM reads. Real ones are a few hundred KB.
pub const MAX_TORRENT_FILE: u64 = 20 * 1024 * 1024;

/// Seed ratio (in percent) that means "share until SDM quits".
pub const SEED_FOREVER: u16 = u16::MAX;

/// Where a torrent comes from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentSource {
    /// A magnet link, or the address of a `.torrent` file.
    #[serde(default)]
    pub link: Option<String>,
    /// A `.torrent` file on this computer.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// Cookies and referer for downloading the `.torrent` file.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
}

/// A file inside a torrent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentFile {
    /// Position in the torrent, used to choose files.
    pub index: usize,
    /// Path inside the torrent, with `/` between folders.
    pub path: String,
    pub size: u64,
}

/// What is in a torrent, for the "add torrent" dialog.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentInfo {
    /// The torrent's v1 info hash in hex.
    pub info_hash: String,
    pub name: String,
    /// Size of all files.
    pub size: u64,
    /// The files, without padding files.
    pub files: Vec<TorrentFile>,
    /// `true` for torrents that only use their own trackers.
    pub private: bool,
    pub comment: Option<String>,
    pub pieces: u32,
    pub piece_length: u32,
    /// Where it will be saved unless the user picks another folder.
    pub directory: PathBuf,
    /// The download that already has this torrent.
    pub existing: Option<u64>,
}

impl TorrentInfo {
    /// `true` when the torrent is a single file rather than a folder.
    pub fn single_file(&self) -> bool {
        self.files.len() == 1 && !self.files[0].path.contains('/')
    }

    /// The selected files, or all of them; `None` when all are selected.
    pub fn selection(&self, wanted: Option<&[usize]>) -> Result<Option<Vec<usize>>, String> {
        let Some(wanted) = wanted else {
            return Ok(None);
        };
        let mut chosen: Vec<usize> = self
            .files
            .iter()
            .map(|f| f.index)
            .filter(|i| wanted.contains(i))
            .collect();
        chosen.sort_unstable();
        chosen.dedup();
        if chosen.is_empty() {
            return Err("choose at least one file".into());
        }
        Ok((chosen.len() < self.files.len()).then_some(chosen))
    }

    /// Bytes to download for a selection.
    pub fn selected_size(&self, selection: Option<&[usize]>) -> u64 {
        self.files
            .iter()
            .filter(|f| selection.is_none_or(|s| s.contains(&f.index)))
            .map(|f| f.size)
            .sum()
    }

    /// The biggest file decides the category (a film with its subtitles
    /// is a video).
    pub fn main_file(&self) -> &str {
        self.files
            .iter()
            .max_by_key(|f| f.size)
            .map_or(self.name.as_str(), |f| f.path.as_str())
    }
}

/// A request to download (some files of) a torrent seen before with
/// [`TorrentEngine::inspect`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTorrentRequest {
    pub info_hash: String,
    /// Indexes of the files to download; all when missing.
    #[serde(default)]
    pub files: Option<Vec<usize>>,
    #[serde(default)]
    pub directory: Option<PathBuf>,
    #[serde(default)]
    pub start: bool,
}

/// How a torrent download is made. Stored with the download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentSpec {
    pub info_hash: String,
    /// Files to download; all when missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<usize>>,
    /// The folder the torrent's files are written into.
    pub folder: PathBuf,
}

/// `true` for `magnet:?...` links.
pub fn is_magnet(link: &str) -> bool {
    link.trim()
        .get(..8)
        .is_some_and(|start| start.eq_ignore_ascii_case("magnet:?"))
}

/// `true` when a link or file name looks like a `.torrent` file.
pub fn is_torrent_file_name(name: &str) -> bool {
    let name = name.split(['?', '#']).next().unwrap_or_default();
    name.len() > 8 && name[name.len() - 8..].eq_ignore_ascii_case(".torrent")
}

/// The info hash of a magnet link, in lowercase hex.
pub fn magnet_info_hash(link: &str) -> Option<String> {
    librqbit::Magnet::parse(link.trim())
        .ok()?
        .as_id20()
        .map(|id| id.as_string())
}

/// A magnet link for a torrent, so every torrent download has a link to
/// copy.
pub fn magnet_link(info_hash: &str, name: &str) -> String {
    let name: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
    format!("magnet:?xt=urn:btih:{info_hash}&dn={name}")
}

/// Reads a `.torrent` file.
pub fn parse(bytes: &[u8]) -> Result<TorrentInfo, String> {
    let meta = librqbit::torrent_from_bytes(bytes)
        .map_err(|error| format!("this is not a valid .torrent file ({error})"))?;
    let info_hash = meta.info_hash.as_string();
    let comment = meta
        .comment
        .as_ref()
        .map(|c| String::from_utf8_lossy(c.as_ref()).trim().to_owned())
        .filter(|c| !c.is_empty());
    let info = meta
        .info
        .data
        .validate()
        .map_err(|error| format!("this .torrent file is broken ({error})"))?;
    let files: Vec<TorrentFile> = info
        .iter_file_details()
        .enumerate()
        .filter(|(_, details)| !details.attrs().padding)
        .map(|(index, details)| TorrentFile {
            index,
            path: details
                .filename
                .iter_components()
                .collect::<Vec<_>>()
                .join("/"),
            size: details.len,
        })
        .collect();
    let name = info
        .name()
        .map(|n| n.into_owned())
        .or_else(|| files.first().map(|f| f.path.clone()))
        .unwrap_or_else(|| info_hash.clone());
    Ok(TorrentInfo {
        size: files.iter().map(|f| f.size).sum(),
        private: info.info().private,
        pieces: info.lengths().total_pieces(),
        piece_length: info.info().piece_length,
        info_hash,
        name,
        files,
        comment,
        directory: PathBuf::new(),
        existing: None,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Bencodes a small torrent with the given files (path, size).
    pub fn torrent_bytes(name: &str, files: &[(&str, u64)], piece_length: u64) -> Vec<u8> {
        fn string(out: &mut Vec<u8>, s: &[u8]) {
            out.extend_from_slice(format!("{}:", s.len()).as_bytes());
            out.extend_from_slice(s);
        }
        let total: u64 = files.iter().map(|f| f.1).sum();
        let pieces = total.div_ceil(piece_length).max(1);
        let mut info = b"d".to_vec();
        if files.len() == 1 && !files[0].0.contains('/') {
            string(&mut info, b"length");
            info.extend_from_slice(format!("i{}e", files[0].1).as_bytes());
        } else {
            string(&mut info, b"files");
            info.push(b'l');
            for (path, size) in files {
                info.extend_from_slice(b"d");
                string(&mut info, b"length");
                info.extend_from_slice(format!("i{size}e").as_bytes());
                string(&mut info, b"path");
                info.push(b'l');
                for part in path.split('/') {
                    string(&mut info, part.as_bytes());
                }
                info.extend_from_slice(b"ee");
            }
            info.push(b'e');
        }
        string(&mut info, b"name");
        string(&mut info, name.as_bytes());
        string(&mut info, b"piece length");
        info.extend_from_slice(format!("i{piece_length}e").as_bytes());
        string(&mut info, b"pieces");
        string(&mut info, &vec![0u8; usize::try_from(pieces).unwrap() * 20]);
        info.push(b'e');

        let mut out = b"d".to_vec();
        string(&mut out, b"comment");
        string(&mut out, b"Test torrent");
        string(&mut out, b"info");
        out.extend_from_slice(&info);
        out.push(b'e');
        out
    }

    #[test]
    fn reads_torrent_files() {
        let bytes = torrent_bytes(
            "Album",
            &[("cd1/01 Intro.mp3", 1000), ("cover.jpg", 300)],
            16384,
        );
        let info = parse(&bytes).unwrap();
        assert_eq!(info.name, "Album");
        assert_eq!(info.size, 1300);
        assert_eq!(info.info_hash.len(), 40);
        assert_eq!(info.comment.as_deref(), Some("Test torrent"));
        assert_eq!(
            info.files,
            vec![
                TorrentFile {
                    index: 0,
                    path: "cd1/01 Intro.mp3".into(),
                    size: 1000
                },
                TorrentFile {
                    index: 1,
                    path: "cover.jpg".into(),
                    size: 300
                },
            ]
        );
        assert!(!info.single_file());
        assert_eq!(info.main_file(), "cd1/01 Intro.mp3");

        let single = parse(&torrent_bytes("movie.mkv", &[("movie.mkv", 5000)], 16384)).unwrap();
        assert!(single.single_file());
        assert_eq!(single.files[0].path, "movie.mkv");

        assert!(parse(b"not a torrent").is_err());
    }

    #[test]
    fn chooses_files() {
        let info = parse(&torrent_bytes(
            "Pack",
            &[("a.bin", 10), ("b.bin", 20), ("c.bin", 30)],
            16384,
        ))
        .unwrap();
        assert_eq!(info.selection(None), Ok(None));
        assert_eq!(info.selection(Some(&[2, 0, 0, 9])), Ok(Some(vec![0, 2])));
        assert_eq!(info.selection(Some(&[0, 1, 2])), Ok(None));
        assert!(info.selection(Some(&[])).is_err());
        assert_eq!(info.selected_size(Some(&[0, 2])), 40);
        assert_eq!(info.selected_size(None), 60);
    }

    #[test]
    fn recognises_links() {
        let link = "magnet:?xt=urn:btih:0123456789ABCDEF0123456789abcdef01234567&dn=Ubuntu";
        assert!(is_magnet(link));
        assert!(is_magnet(" MAGNET:?xt=urn:btih:x"));
        assert!(!is_magnet("https://example.com/magnet:?"));
        assert_eq!(
            magnet_info_hash(link).as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(magnet_info_hash("magnet:?dn=nothing"), None);
        assert!(is_torrent_file_name(
            "https://x.org/ubuntu.iso.torrent?key=1"
        ));
        assert!(is_torrent_file_name("Ubuntu.TORRENT"));
        assert!(!is_torrent_file_name("https://x.org/torrent"));
        assert_eq!(
            magnet_link("abc", "Big Buck Bunny"),
            "magnet:?xt=urn:btih:abc&dn=Big+Buck+Bunny"
        );
    }
}
