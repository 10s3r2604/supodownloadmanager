//! Choosing safe, readable file names for downloads.

use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;
use reqwest::Url;

/// Name used when neither the server nor the URL suggests one.
pub(crate) const FALLBACK_NAME: &str = "download";

/// Longest file name (in bytes) we create; most file systems allow 255.
const MAX_NAME_BYTES: usize = 200;

/// Extracts the file name from a `Content-Disposition` header value,
/// preferring the RFC 5987 `filename*` form when present.
pub(crate) fn from_content_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    let mut extended = None;
    for param in split_params(value) {
        let Some((key, val)) = param.split_once('=') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "filename*" => extended = decode_extended(val.trim()),
            "filename" => plain = Some(unquote(val.trim())),
            _ => {}
        }
    }
    extended.or(plain).filter(|name| !name.trim().is_empty())
}

/// Uses the last non-empty path segment of the URL as file name.
pub(crate) fn from_url(url: &Url) -> Option<String> {
    let segment = url.path_segments()?.rfind(|s| !s.is_empty())?;
    let decoded = percent_decode_str(segment).decode_utf8_lossy().into_owned();
    (!decoded.trim().is_empty()).then_some(decoded)
}

/// Turns any string into a file name that is valid on Windows, macOS and
/// Linux.
pub fn sanitize(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    // Windows silently drops trailing dots and spaces.
    let trimmed = replaced.trim().trim_end_matches(['.', ' ']);
    if trimmed.is_empty() {
        return FALLBACK_NAME.to_owned();
    }

    let mut name = trimmed.to_owned();
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if is_reserved_on_windows(&stem) {
        name.insert(0, '_');
    }
    truncate(name)
}

fn is_reserved_on_windows(stem: &str) -> bool {
    match stem {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => {
            let bytes = stem.as_bytes();
            bytes.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && (b'1'..=b'9').contains(&bytes[3])
        }
    }
}

fn truncate(name: String) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name;
    }
    let (stem, ext) = split_extension(&name);
    let mut end = MAX_NAME_BYTES.saturating_sub(ext.len()).min(stem.len());
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{ext}", &stem[..end])
}

/// Splits `archive.tar.gz` into `("archive", ".tar.gz")` and `notes.txt`
/// into `("notes", ".txt")`.
pub(crate) fn split_extension(name: &str) -> (&str, &str) {
    let Some(dot) = name.rfind('.').filter(|&i| i > 0) else {
        return (name, "");
    };
    let ext = &name[dot..];
    if ext.len() > 10 || ext.contains(' ') {
        return (name, "");
    }
    let stem = &name[..dot];
    if let Some(tar) = stem.rfind('.').filter(|&i| i > 0)
        && stem[tar..].eq_ignore_ascii_case(".tar")
    {
        return name.split_at(tar);
    }
    (stem, ext)
}

/// Yields `dir/name`, `dir/name (1)`, `dir/name (2)`, ... keeping the
/// extension at the end. Useful for picking a name that is not taken yet.
pub fn candidates(dir: &Path, name: &str) -> impl Iterator<Item = PathBuf> + use<> {
    let dir = dir.to_owned();
    let (stem, ext) = split_extension(name);
    let (name, stem, ext) = (name.to_owned(), stem.to_owned(), ext.to_owned());
    (0..10_000).map(move |i| match i {
        0 => dir.join(&name),
        i => dir.join(format!("{stem} ({i}){ext}")),
    })
}

/// Splits header parameters on `;`, ignoring separators inside quotes.
fn split_params(value: &str) -> Vec<&str> {
    let mut params = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaped = false;
    for (i, c) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => {
                params.push(&value[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    params.push(&value[start..]);
    params
}

fn unquote(value: &str) -> String {
    let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        return value.to_owned();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            c => out.push(c),
        }
    }
    out
}

/// Decodes an RFC 5987 value such as `UTF-8''na%C3%AFve.txt`.
fn decode_extended(value: &str) -> Option<String> {
    let value = unquote(value);
    let mut parts = value.splitn(3, '\'');
    let charset = parts.next()?;
    let _language = parts.next()?;
    let bytes: Vec<u8> = percent_decode_str(parts.next()?).collect();
    if charset.eq_ignore_ascii_case("utf-8") {
        Some(String::from_utf8_lossy(&bytes).into_owned())
    } else {
        // ISO-8859-1 maps every byte to the code point of the same value.
        Some(bytes.iter().map(|&b| char::from(b)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition_plain() {
        assert_eq!(
            from_content_disposition("attachment; filename=report.pdf").as_deref(),
            Some("report.pdf")
        );
        assert_eq!(
            from_content_disposition(r#"attachment; filename="my; file \"1\".zip""#).as_deref(),
            Some(r#"my; file "1".zip"#)
        );
    }

    #[test]
    fn content_disposition_extended_wins() {
        let header =
            r#"attachment; filename="fallback.pdf"; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf"#;
        assert_eq!(
            from_content_disposition(header).as_deref(),
            Some("résumé.pdf")
        );
        assert_eq!(
            from_content_disposition("attachment; filename*=iso-8859-1'en'%E9t%E9.txt").as_deref(),
            Some("été.txt")
        );
    }

    #[test]
    fn content_disposition_without_name() {
        assert_eq!(from_content_disposition("inline"), None);
        assert_eq!(from_content_disposition(r#"attachment; filename="""#), None);
    }

    #[test]
    fn name_from_url() {
        let url = Url::parse("https://example.com/files/Big%20File.iso?token=1").unwrap();
        assert_eq!(from_url(&url).as_deref(), Some("Big File.iso"));
        let url = Url::parse("https://example.com/dir/").unwrap();
        assert_eq!(from_url(&url).as_deref(), Some("dir"));
        let url = Url::parse("https://example.com/").unwrap();
        assert_eq!(from_url(&url), None);
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize("a/b\\c:d*e?f\"g<h>i|j"), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanitize("  name.txt. . "), "name.txt");
        assert_eq!(sanitize(".."), FALLBACK_NAME);
        assert_eq!(sanitize(""), FALLBACK_NAME);
        assert_eq!(sanitize("CON.txt"), "_CON.txt");
        assert_eq!(sanitize("com1"), "_com1");
        assert_eq!(sanitize("COM10.txt"), "COM10.txt");
        assert_eq!(sanitize("tab\there"), "tab_here");
    }

    #[test]
    fn truncates_long_names_keeping_extension() {
        let long = format!("{}.mp4", "é".repeat(300));
        let name = sanitize(&long);
        assert!(name.len() <= MAX_NAME_BYTES);
        assert!(name.ends_with(".mp4"));
    }

    #[test]
    fn splits_extensions() {
        assert_eq!(split_extension("notes.txt"), ("notes", ".txt"));
        assert_eq!(split_extension("archive.tar.gz"), ("archive", ".tar.gz"));
        assert_eq!(split_extension(".bashrc"), (".bashrc", ""));
        assert_eq!(split_extension("README"), ("README", ""));
        assert_eq!(
            split_extension("Report v1.2 final"),
            ("Report v1.2 final", "")
        );
    }

    #[test]
    fn numbered_candidates() {
        let dir = Path::new("out");
        let names: Vec<_> = candidates(dir, "a.tar.gz").take(3).collect();
        assert_eq!(
            names,
            [
                dir.join("a.tar.gz"),
                dir.join("a (1).tar.gz"),
                dir.join("a (2).tar.gz")
            ]
        );
    }
}
