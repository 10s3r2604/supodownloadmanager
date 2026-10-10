//! Asking the server about a file before downloading it.

use reqwest::header::{self, HeaderMap};
use reqwest::{Client, StatusCode, Url};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::filename;

/// What the server told us about a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    /// The URL that was requested.
    pub url: String,
    /// The URL after following redirects.
    pub final_url: String,
    /// Size in bytes, if the server reported it.
    pub size: Option<u64>,
    /// Whether the server accepts byte range requests. Only then can a
    /// download use several connections and be resumed.
    pub resumable: bool,
    /// Suggested file name, already safe to use on any operating system.
    pub file_name: String,
    /// The `Content-Type` reported by the server.
    pub content_type: Option<String>,
    /// The `ETag` reported by the server.
    pub etag: Option<String>,
    /// The `Last-Modified` date reported by the server.
    pub last_modified: Option<String>,
}

/// A parsed `Content-Range` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContentRange {
    /// First and last (inclusive) byte, `None` for `bytes */total`.
    pub range: Option<(u64, u64)>,
    /// Complete length, `None` when the server sent `*`.
    pub total: Option<u64>,
}

pub(crate) fn parse_content_range(value: &str) -> Option<ContentRange> {
    let rest = value.trim().strip_prefix("bytes")?;
    let (range, total) = rest.trim_start().split_once('/')?;
    let total = match total.trim() {
        "*" => None,
        total => Some(total.parse().ok()?),
    };
    let range = match range.trim() {
        "*" => None,
        range => {
            let (first, last) = range.split_once('-')?;
            let first: u64 = first.trim().parse().ok()?;
            let last: u64 = last.trim().parse().ok()?;
            if last < first {
                return None;
            }
            Some((first, last))
        }
    };
    Some(ContentRange { range, total })
}

/// Requests the first byte of the file. Compared to `HEAD`, this tells us
/// reliably whether ranges work and is handled correctly by more servers.
pub(crate) async fn probe(client: &Client, url: &Url, headers: &HeaderMap) -> Result<RemoteInfo> {
    let response = client
        .get(url.clone())
        .headers(headers.clone())
        .header(header::RANGE, "bytes=0-0")
        .send()
        .await?;

    let status = response.status();
    let headers = response.headers();
    let text = |name: header::HeaderName| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let content_range = text(header::CONTENT_RANGE).and_then(|v| parse_content_range(&v));

    let (size, resumable) = match status {
        StatusCode::PARTIAL_CONTENT => match content_range {
            Some(ContentRange {
                range: Some((0, _)),
                total,
            }) => (total, total.is_some()),
            _ => (None, false),
        },
        // Asking for the first byte of an empty file is "not satisfiable".
        StatusCode::RANGE_NOT_SATISFIABLE => match content_range {
            Some(ContentRange { total: Some(0), .. }) => (Some(0), false),
            _ => return Err(Error::Status(status)),
        },
        status if status.is_success() => (response.content_length(), false),
        status => return Err(Error::Status(status)),
    };

    let suggested = headers
        .get(header::CONTENT_DISPOSITION)
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
        .and_then(|v| filename::from_content_disposition(&v))
        .or_else(|| filename::from_url(response.url()))
        .unwrap_or_default();

    Ok(RemoteInfo {
        url: url.to_string(),
        final_url: response.url().to_string(),
        size,
        resumable,
        file_name: filename::sanitize(&suggested),
        content_type: text(header::CONTENT_TYPE),
        etag: text(header::ETAG),
        last_modified: text(header::LAST_MODIFIED),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_range() {
        assert_eq!(
            parse_content_range("bytes 0-0/1234"),
            Some(ContentRange {
                range: Some((0, 0)),
                total: Some(1234)
            })
        );
        assert_eq!(
            parse_content_range("bytes 100-199/*"),
            Some(ContentRange {
                range: Some((100, 199)),
                total: None
            })
        );
        assert_eq!(
            parse_content_range("bytes */0"),
            Some(ContentRange {
                range: None,
                total: Some(0)
            })
        );
        assert_eq!(parse_content_range("bytes 5-1/10"), None);
        assert_eq!(parse_content_range("items 0-1/2"), None);
        assert_eq!(parse_content_range("bytes 0-x/2"), None);
    }
}

#[cfg(test)]
mod serialization_tests {
    use super::*;

    /// User interfaces written in JavaScript expect camelCase names.
    #[test]
    fn serializes_camel_case() {
        let info = RemoteInfo {
            url: "u".into(),
            final_url: "f".into(),
            size: Some(1),
            resumable: true,
            file_name: "a.zip".into(),
            content_type: None,
            etag: None,
            last_modified: None,
        };
        let json = serde_json::to_value(info).unwrap();
        assert_eq!(json["fileName"], "a.zip");
        assert_eq!(json["finalUrl"], "f");
        assert!(json.get("file_name").is_none());
    }
}
