use std::path::PathBuf;

use reqwest::StatusCode;

/// Result type used throughout `sdm-core`.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong while downloading.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The URL could not be parsed or does not use `http`/`https`.
    #[error("invalid URL `{0}`")]
    InvalidUrl(String),

    /// A user supplied request header is malformed.
    #[error("invalid header `{0}`")]
    InvalidHeader(String),

    /// A network level error (DNS, connect, TLS, timeout, broken connection, ...).
    #[error("network error: {}", describe(.0))]
    Http(reqwest::Error),

    /// The server answered with an error status.
    #[error("server responded with {0}")]
    Status(StatusCode),

    /// The server answered a byte range request with the whole file.
    #[error("server ignored the byte range request")]
    RangeIgnored,

    /// The server answered with a byte range other than the one requested.
    #[error("server returned an unexpected byte range")]
    BadContentRange,

    /// The file on the server changed while it was being downloaded.
    #[error("the file on the server changed during the download")]
    RemoteChanged,

    /// The connection ended before all expected bytes arrived.
    #[error("connection closed before the data was complete")]
    UnexpectedEof,

    /// The destination file exists and overwriting was not allowed.
    #[error("file already exists: {}", .0.display())]
    FileExists(PathBuf),

    /// Another download in this process is already writing to this file.
    #[error("file is already being downloaded: {}", .0.display())]
    InUse(PathBuf),

    /// Reading or writing a local file failed.
    #[error("I/O error: {0}")]
    Io(std::io::Error),

    /// The saved progress of a download could not be read.
    #[error("corrupt download state: {0}")]
    State(String),

    /// The operation was stopped because the download was paused or cancelled.
    #[error("the download was stopped")]
    Cancelled,

    /// A file meant to be read into memory is bigger than allowed.
    #[error("the file is larger than {0} bytes")]
    TooLarge(u64),
}

// The wrapped errors are part of the message instead of being reported as
// `source()`, so that printing the error chain does not repeat them.
impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        // The URL is known to the caller and can be very long (signed links).
        Error::Http(error.without_url())
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error)
    }
}

/// Renders an error with its causes, e.g.
/// `error sending request: client error (Connect): dns error: no such host`.
fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

impl Error {
    /// Whether trying the same request again may succeed.
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::Http(e) => {
                !(e.is_builder() || e.is_redirect()) && e.status().is_none_or(is_retryable_status)
            }
            Error::Status(status) => is_retryable_status(*status),
            Error::UnexpectedEof | Error::BadContentRange => true,
            _ => false,
        }
    }
}

fn is_retryable_status(status: StatusCode) -> bool {
    status.is_server_error()
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_statuses() {
        assert!(Error::Status(StatusCode::SERVICE_UNAVAILABLE).is_retryable());
        assert!(Error::Status(StatusCode::TOO_MANY_REQUESTS).is_retryable());
        assert!(!Error::Status(StatusCode::NOT_FOUND).is_retryable());
        assert!(!Error::Status(StatusCode::FORBIDDEN).is_retryable());
        assert!(Error::UnexpectedEof.is_retryable());
        assert!(!Error::RangeIgnored.is_retryable());
    }
}
