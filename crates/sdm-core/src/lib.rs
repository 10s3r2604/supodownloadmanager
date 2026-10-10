//! The download engine behind **Supo Download Manager (SDM)**.
//!
//! `sdm-core` downloads files over HTTP(S) using several connections at once,
//! keeps every connection busy by splitting the remaining work on the fly,
//! retries on network errors and can pause and resume downloads, even after
//! the program was closed.
//!
//! ```no_run
//! use sdm_core::{ClientConfig, Destination, DownloadRequest, Downloader, Outcome};
//!
//! # async fn demo() -> sdm_core::Result<()> {
//! let downloader = Downloader::new(ClientConfig::default())?;
//! let handle = downloader.start(DownloadRequest::new(
//!     "https://example.com/big.iso",
//!     Destination::Directory("downloads".into()),
//! ));
//! if let Outcome::Completed { path, .. } = handle.wait().await? {
//!     println!("saved to {}", path.display());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! While a download runs, [`DownloadHandle::progress`] returns a snapshot that
//! a user interface can render, and [`DownloadHandle::pause`] stops it while
//! keeping everything downloaded so far. Starting the same download again
//! continues where it stopped.

#![forbid(unsafe_code)]

mod checksum;
mod engine;
mod error;
mod filename;
mod limiter;
mod probe;
mod segment;
mod state;

pub use checksum::sha256_file;
pub use engine::{
    ClientConfig, Destination, DownloadControl, DownloadHandle, DownloadOptions, DownloadRequest,
    Downloader, Outcome, Phase, Progress, ProxyConfig, SegmentProgress,
};
pub use error::{Error, Result};
pub use filename::sanitize as sanitize_file_name;
pub use probe::RemoteInfo;
