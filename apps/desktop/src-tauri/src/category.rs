//! Sorting downloads into categories by file type, like IDM does.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Video,
    Music,
    Document,
    Compressed,
    Program,
    Image,
    Other,
}

const VIDEO: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "3gp", "ts", "m2ts",
    "vob", "rmvb",
];
const MUSIC: &[&str] = &[
    "mp3", "wav", "flac", "aac", "ogg", "oga", "m4a", "wma", "opus", "aiff", "ape",
];
const DOCUMENT: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt", "csv",
    "epub", "mobi",
];
const COMPRESSED: &[&str] = &[
    "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "iso", "img", "cab",
];
const PROGRAM: &[&str] = &[
    "exe", "msi", "msix", "dmg", "pkg", "deb", "rpm", "apk", "appimage", "flatpak", "bin", "run",
];
const IMAGE: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "tif", "tiff", "heic", "avif", "psd",
];

impl Category {
    /// Guesses the category from a file name's extension.
    pub fn from_file_name(name: &str) -> Self {
        let Some((_, ext)) = name.rsplit_once('.') else {
            return Category::Other;
        };
        let ext = ext.to_ascii_lowercase();
        let ext = ext.as_str();
        [
            (VIDEO, Category::Video),
            (MUSIC, Category::Music),
            (DOCUMENT, Category::Document),
            (COMPRESSED, Category::Compressed),
            (PROGRAM, Category::Program),
            (IMAGE, Category::Image),
        ]
        .into_iter()
        .find(|(list, _)| list.contains(&ext))
        .map_or(Category::Other, |(_, category)| category)
    }

    /// Sub-folder of the download folder used for this category.
    pub fn folder(self) -> Option<&'static str> {
        match self {
            Category::Video => Some("Video"),
            Category::Music => Some("Music"),
            Category::Document => Some("Documents"),
            Category::Compressed => Some("Compressed"),
            Category::Program => Some("Programs"),
            Category::Image => Some("Images"),
            Category::Other => None,
        }
    }
}

/// File types the browser extension hands over to SDM by default.
pub fn default_capture_extensions() -> Vec<String> {
    [VIDEO, MUSIC, COMPRESSED, PROGRAM, &["pdf", "epub"]]
        .concat()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorises_by_extension() {
        assert_eq!(Category::from_file_name("movie.MKV"), Category::Video);
        assert_eq!(Category::from_file_name("song.mp3"), Category::Music);
        assert_eq!(Category::from_file_name("report.pdf"), Category::Document);
        assert_eq!(
            Category::from_file_name("backup.tar.gz"),
            Category::Compressed
        );
        assert_eq!(Category::from_file_name("setup.exe"), Category::Program);
        assert_eq!(Category::from_file_name("photo.jpeg"), Category::Image);
        assert_eq!(Category::from_file_name("README"), Category::Other);
        assert_eq!(Category::from_file_name("page.html"), Category::Other);
    }

    #[test]
    fn capture_list_skips_images_and_pages() {
        let list = default_capture_extensions();
        assert!(list.contains(&"mp4".to_owned()));
        assert!(list.contains(&"zip".to_owned()));
        assert!(!list.contains(&"png".to_owned()));
        assert!(!list.contains(&"html".to_owned()));
    }
}
