//! Noticing download links copied to the clipboard.

/// Returns the link when `text` is a magnet link, or a single http(s) URL
/// that points to a file type the user wants SDM to handle.
pub fn downloadable_link(text: &str, extensions: &[String]) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.len() > 8192 || text.contains(char::is_whitespace) {
        return None;
    }
    if crate::torrent::is_magnet(text) {
        return crate::torrent::magnet_info_hash(text).map(|_| text.to_owned());
    }
    let url = url::Url::parse(text).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let last = url.path_segments()?.rfind(|s| !s.is_empty())?;
    let (_, ext) = last.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    extensions.contains(&ext).then(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exts() -> Vec<String> {
        vec!["zip".into(), "mp4".into()]
    }

    #[test]
    fn finds_file_links() {
        assert_eq!(
            downloadable_link(" https://example.com/files/a.ZIP?x=1 ", &exts()).as_deref(),
            Some("https://example.com/files/a.ZIP?x=1")
        );
        assert!(downloadable_link("http://example.com/v/clip.mp4", &exts()).is_some());
        let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=x";
        assert_eq!(downloadable_link(magnet, &exts()).as_deref(), Some(magnet));
        assert_eq!(downloadable_link("magnet:?dn=no-hash", &exts()), None);
    }

    #[test]
    fn ignores_everything_else() {
        for text in [
            "",
            "hello world",
            "https://example.com/",
            "https://example.com/page.html",
            "ftp://example.com/a.zip",
            "see https://example.com/a.zip",
            "https://example.com/a.zip\nhttps://example.com/b.zip",
        ] {
            assert_eq!(downloadable_link(text, &exts()), None, "{text:?}");
        }
    }
}
