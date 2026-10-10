//! Helpers for tests that share and download real torrents on this
//! computer.

use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use librqbit::spawn_utils::BlockingSpawner;
use librqbit::{CreateTorrentOptions, create_torrent};

/// Data that differs from byte to byte, so misplaced pieces show.
pub fn data(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| ((i as u32).wrapping_mul(2_654_435_761) >> 13) as u8 ^ seed)
        .collect()
}

pub async fn make_torrent(folder: &Path, tracker: &str) -> Vec<u8> {
    let options = CreateTorrentOptions {
        name: None,
        trackers: vec![tracker.to_owned()],
        piece_length: Some(32 * 1024),
    };
    let created = create_torrent(folder, options, &BlockingSpawner::new(2))
        .await
        .unwrap();
    created.as_bytes().unwrap().to_vec()
}

/// A tiny HTTP tracker: every peer that announces is told about all
/// the others. Returns its announce URL.
pub async fn tracker() -> String {
    use axum::extract::{RawQuery, State};
    use axum::routing::get;

    type Peers = Arc<Mutex<Vec<u16>>>;
    async fn announce(State(peers): State<Peers>, RawQuery(query): RawQuery) -> Vec<u8> {
        let port = query
            .unwrap_or_default()
            .split('&')
            .find_map(|pair| pair.strip_prefix("port="))
            .and_then(|p| p.parse::<u16>().ok());
        let mut peers = peers.lock().unwrap();
        if let Some(port) = port
            && !peers.contains(&port)
        {
            peers.push(port);
        }
        let mut compact = Vec::new();
        for port in peers.iter() {
            compact.extend_from_slice(&[127, 0, 0, 1]);
            compact.extend_from_slice(&port.to_be_bytes());
        }
        let mut body = format!("d8:intervali2e5:peers{}:", compact.len()).into_bytes();
        body.extend_from_slice(&compact);
        body.push(b'e');
        body
    }

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new()
        .route("/announce", get(announce))
        .with_state(Peers::default());
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{addr}/announce")
}
