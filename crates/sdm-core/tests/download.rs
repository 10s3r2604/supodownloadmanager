//! End-to-end tests against a local HTTP server that can misbehave in the
//! ways real servers do.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use sdm_core::{
    ClientConfig, Destination, DownloadHandle, DownloadOptions, DownloadRequest, Downloader, Error,
    Outcome, ProxyConfig,
};
use tokio::time::Instant;

/// Test server. The first path segment picks its behaviour:
///
/// * `file`: well behaved, supports byte ranges
/// * `named`: like `file`, plus a `Content-Disposition` header
/// * `norange`: ignores byte ranges
/// * `chunked`: ignores byte ranges and does not send a length
/// * `flaky`: supports ranges, but the first few responses break halfway
/// * `lying`: claims range support for the first byte only
/// * anything else: 404
struct Server {
    data: Vec<u8>,
    failures_left: AtomicUsize,
    data_requests: AtomicUsize,
}

async fn start_server(data: Vec<u8>, failures: usize) -> (String, Arc<Server>) {
    let server = Arc::new(Server {
        data,
        failures_left: AtomicUsize::new(failures),
        data_requests: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route("/{mode}/{name}", get(serve))
        .with_state(server.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), server)
}

fn parse_range(value: &str) -> Option<(u64, Option<u64>)> {
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    let end = if end.is_empty() {
        None
    } else {
        Some(end.parse().ok()?)
    };
    Some((start.parse().ok()?, end))
}

async fn serve(
    State(server): State<Arc<Server>>,
    UrlPath((mode, _name)): UrlPath<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let len = server.data.len() as u64;
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_range);
    let is_probe = range == Some((0, Some(0)));
    let honours_range = match mode.as_str() {
        "file" | "named" | "flaky" => true,
        "lying" => is_probe,
        "norange" | "chunked" => false,
        _ => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .unwrap();
        }
    };
    if !is_probe {
        server.data_requests.fetch_add(1, Ordering::SeqCst);
    }

    let mut response = Response::builder().header(header::ETAG, "\"v1\"");
    if mode == "named" {
        response = response.header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"fallback.pdf\"; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf",
        );
    }

    let (start, end) = match range {
        Some((start, _)) if honours_range && start >= len => {
            return response
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{len}"))
                .body(Body::empty())
                .unwrap();
        }
        Some((start, end)) if honours_range => {
            let end = end.map_or(len, |e| (e + 1).min(len));
            response = response.status(StatusCode::PARTIAL_CONTENT).header(
                header::CONTENT_RANGE,
                format!("bytes {start}-{}/{len}", end - 1),
            );
            (start, end)
        }
        _ => {
            response = response.status(StatusCode::OK);
            (0, len)
        }
    };
    let body = Bytes::copy_from_slice(&server.data[start as usize..end as usize]);

    if mode == "chunked" {
        let chunks: Vec<Result<Bytes, std::io::Error>> = body
            .chunks(10_000)
            .map(|c| Ok(Bytes::copy_from_slice(c)))
            .collect();
        return response
            .body(Body::from_stream(futures_util::stream::iter(chunks)))
            .unwrap();
    }

    response = response.header(header::CONTENT_LENGTH, body.len());
    if mode == "flaky" && !is_probe && body.len() > 2 && take_failure(&server) {
        let cut = (body.len() / 2).min(64 * 1024);
        let parts: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(body.slice(..cut)),
            Err(std::io::Error::other("connection dropped")),
        ];
        return response
            .body(Body::from_stream(futures_util::stream::iter(parts)))
            .unwrap();
    }
    response.body(Body::from(body)).unwrap()
}

fn take_failure(server: &Server) -> bool {
    let mut left = server.failures_left.load(Ordering::SeqCst);
    while left > 0 {
        match server.failures_left.compare_exchange(
            left,
            left - 1,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => return true,
            Err(current) => left = current,
        }
    }
    false
}

/// Deterministic pseudo-random test data.
fn test_data(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

fn downloader() -> Downloader {
    Downloader::new(ClientConfig {
        proxy: ProxyConfig::Disabled,
        ..ClientConfig::default()
    })
    .unwrap()
}

fn options() -> DownloadOptions {
    DownloadOptions {
        connections: 4,
        min_segment_size: 64 * 1024,
        retry_delay: Duration::from_millis(10),
        ..DownloadOptions::default()
    }
}

fn request(url: String, destination: Destination) -> DownloadRequest {
    DownloadRequest::new(url, destination).options(options())
}

async fn completed(handle: DownloadHandle) -> (std::path::PathBuf, u64) {
    match tokio::time::timeout(Duration::from_secs(60), handle.wait())
        .await
        .expect("download timed out")
    {
        Ok(Outcome::Completed { path, size }) => (path, size),
        other => panic!("download did not complete: {other:?}"),
    }
}

async fn wait_for_bytes(handle: &DownloadHandle, bytes: u64) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while handle.progress().downloaded < bytes {
        assert!(Instant::now() < deadline, "download made no progress");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".sdm-"))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_with_several_connections() {
    let data = test_data(3 * 1024 * 1024 + 123);
    let (base, server) = start_server(data.clone(), 0).await;
    let dir = tempfile::tempdir().unwrap();

    let handle = downloader().start(request(
        format!("{base}/file/data.bin"),
        Destination::Directory(dir.path().into()),
    ));
    let (path, size) = completed(handle).await;

    assert_eq!(path, dir.path().join("data.bin"));
    assert_eq!(size, data.len() as u64);
    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert!(server.data_requests.load(Ordering::SeqCst) >= 4);
    assert!(leftovers(dir.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn handles_servers_without_range_support() {
    let data = test_data(700 * 1024);
    let (base, _server) = start_server(data.clone(), 0).await;
    let dir = tempfile::tempdir().unwrap();

    let handle = downloader().start(request(
        format!("{base}/norange/data.bin"),
        Destination::Directory(dir.path().into()),
    ));
    let (path, _) = completed(handle).await;
    assert_eq!(std::fs::read(path).unwrap(), data);
    assert!(leftovers(dir.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn handles_unknown_size() {
    let data = test_data(500 * 1024 + 7);
    let (base, _server) = start_server(data.clone(), 0).await;
    let dir = tempfile::tempdir().unwrap();

    let handle = downloader().start(request(
        format!("{base}/chunked/data.bin"),
        Destination::Directory(dir.path().into()),
    ));
    let (path, size) = completed(handle).await;
    assert_eq!(size, data.len() as u64);
    assert_eq!(std::fs::read(path).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retries_broken_connections() {
    let data = test_data(1024 * 1024);
    let (base, server) = start_server(data.clone(), 6).await;
    let dir = tempfile::tempdir().unwrap();

    let handle = downloader().start(request(
        format!("{base}/flaky/data.bin"),
        Destination::Directory(dir.path().into()),
    ));
    let (path, _) = completed(handle).await;
    assert_eq!(std::fs::read(path).unwrap(), data);
    assert_eq!(server.failures_left.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn falls_back_when_server_ignores_ranges() {
    let data = test_data(2 * 1024 * 1024);
    let (base, _server) = start_server(data.clone(), 0).await;
    let dir = tempfile::tempdir().unwrap();

    let handle = downloader().start(request(
        format!("{base}/lying/data.bin"),
        Destination::Directory(dir.path().into()),
    ));
    let (path, _) = completed(handle).await;
    assert_eq!(std::fs::read(path).unwrap(), data);
    assert!(leftovers(dir.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pauses_and_resumes() {
    let data = test_data(2 * 1024 * 1024);
    let (base, _server) = start_server(data.clone(), 0).await;
    let dir = tempfile::tempdir().unwrap();
    let url = format!("{base}/file/data.bin");
    let downloader = downloader();

    let mut slow = request(url.clone(), Destination::Directory(dir.path().into()));
    slow.options.speed_limit = Some(1024 * 1024);
    let handle = downloader.start(slow);
    wait_for_bytes(&handle, 300 * 1024).await;
    handle.pause();
    let control = handle.control();
    let outcome = handle.wait().await.unwrap();
    let kept = control.progress().downloaded;
    assert!(kept >= 300 * 1024);
    assert_eq!(
        outcome,
        Outcome::Paused {
            path: Some(dir.path().join("data.bin")),
            resumable: true
        }
    );
    assert!(dir.path().join("data.bin.sdm-part").exists());
    assert!(dir.path().join("data.bin.sdm-state").exists());
    assert!(!dir.path().join("data.bin").exists());

    let handle = downloader.start(request(url, Destination::Directory(dir.path().into())));
    let control = handle.control();
    let (path, _) = completed(handle).await;
    assert_eq!(path, dir.path().join("data.bin"));
    assert_eq!(std::fs::read(path).unwrap(), data);
    assert!(leftovers(dir.path()).is_empty());

    // The second run started from everything the first one kept.
    assert_eq!(control.progress().resumed_from, kept);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_removes_partial_data() {
    let data = test_data(2 * 1024 * 1024);
    let (base, _server) = start_server(data, 0).await;
    let dir = tempfile::tempdir().unwrap();

    let mut slow = request(
        format!("{base}/file/data.bin"),
        Destination::Directory(dir.path().into()),
    );
    slow.options.speed_limit = Some(512 * 1024);
    let handle = downloader().start(slow);
    wait_for_bytes(&handle, 100 * 1024).await;
    handle.cancel();
    assert_eq!(handle.wait().await.unwrap(), Outcome::Cancelled);
    assert!(leftovers(dir.path()).is_empty());
    assert!(!dir.path().join("data.bin").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn never_overwrites_existing_files_by_default() {
    let data = test_data(100 * 1024);
    let (base, _server) = start_server(data.clone(), 0).await;
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("data.bin");
    std::fs::write(&existing, b"keep me").unwrap();
    let url = format!("{base}/file/data.bin");
    let downloader = downloader();

    // Saving into a directory picks a free name.
    let handle = downloader.start(request(
        url.clone(),
        Destination::Directory(dir.path().into()),
    ));
    let (path, _) = completed(handle).await;
    assert_eq!(path, dir.path().join("data (1).bin"));
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep me");

    // Saving to an explicit file refuses ...
    let handle = downloader.start(request(url.clone(), Destination::File(existing.clone())));
    assert!(matches!(handle.wait().await, Err(Error::FileExists(_))));

    // ... unless overwriting is allowed.
    let mut replace = request(url, Destination::File(existing.clone()));
    replace.options.overwrite = true;
    let (path, _) = completed(downloader.start(replace)).await;
    assert_eq!(path, existing);
    assert_eq!(std::fs::read(&existing).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_file_twice_at_once_gets_two_names() {
    let data = test_data(600 * 1024);
    let (base, _server) = start_server(data.clone(), 0).await;
    let dir = tempfile::tempdir().unwrap();
    let url = format!("{base}/file/data.bin");
    let downloader = downloader();

    let first = downloader.start(request(
        url.clone(),
        Destination::Directory(dir.path().into()),
    ));
    let second = downloader.start(request(url, Destination::Directory(dir.path().into())));
    let (a, _) = completed(first).await;
    let (b, _) = completed(second).await;
    assert_ne!(a, b);
    assert_eq!(std::fs::read(a).unwrap(), data);
    assert_eq!(std::fs::read(b).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn uses_name_from_content_disposition() {
    let data = test_data(10 * 1024);
    let (base, _server) = start_server(data.clone(), 0).await;
    let dir = tempfile::tempdir().unwrap();

    let handle = downloader().start(request(
        format!("{base}/named/get"),
        Destination::Directory(dir.path().into()),
    ));
    let (path, _) = completed(handle).await;
    assert_eq!(path, dir.path().join("résumé.pdf"));
    assert_eq!(std::fs::read(path).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_empty_files() {
    let (base, _server) = start_server(Vec::new(), 0).await;
    let dir = tempfile::tempdir().unwrap();

    let handle = downloader().start(request(
        format!("{base}/file/empty.txt"),
        Destination::Directory(dir.path().into()),
    ));
    let (path, size) = completed(handle).await;
    assert_eq!(size, 0);
    assert_eq!(std::fs::read(path).unwrap(), b"");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reports_missing_files() {
    let (base, _server) = start_server(test_data(10), 0).await;
    let dir = tempfile::tempdir().unwrap();

    let handle = downloader().start(request(
        format!("{base}/missing/data.bin"),
        Destination::Directory(dir.path().into()),
    ));
    let control = handle.control();
    match handle.wait().await {
        Err(Error::Status(status)) => assert_eq!(status.as_u16(), 404),
        other => panic!("expected a 404 error, got {other:?}"),
    }
    assert_eq!(control.progress().phase, sdm_core::Phase::Failed);
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probe_describes_the_file() {
    let data = test_data(12345);
    let (base, _server) = start_server(data, 0).await;

    let info = downloader()
        .probe(&format!("{base}/file/report%20final.pdf"), &[])
        .await
        .unwrap();
    assert_eq!(info.size, Some(12345));
    assert!(info.resumable);
    assert_eq!(info.file_name, "report final.pdf");
    assert_eq!(info.etag.as_deref(), Some("\"v1\""));

    let info = downloader()
        .probe(&format!("{base}/norange/data.bin"), &[])
        .await
        .unwrap();
    assert_eq!(info.size, Some(12345));
    assert!(!info.resumable);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fetches_a_byte_range() {
    let data = test_data(300 * 1024);
    let (base, _server) = start_server(data.clone(), 0).await;
    let mut seen = 0;
    let part = downloader()
        .fetch_range(&format!("{base}/file/data.bin"), 1000..150_000, |n| {
            seen = n
        })
        .await
        .unwrap();
    assert_eq!(part, &data[1000..150_000]);
    assert_eq!(seen, part.len() as u64);

    let ignored = downloader()
        .fetch_range(&format!("{base}/norange/data.bin"), 1000..2000, |_| {})
        .await;
    assert!(matches!(ignored, Err(Error::RangeIgnored)), "{ignored:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fetches_small_files_into_memory() {
    let data = test_data(40 * 1024);
    let (base, _server) = start_server(data.clone(), 0).await;
    let fetched = downloader()
        .fetch(&format!("{base}/file/data.bin"), &[], 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(fetched.data, data);

    let too_big = downloader()
        .fetch(&format!("{base}/file/data.bin"), &[], 1000)
        .await;
    assert!(matches!(too_big, Err(Error::TooLarge(1000))), "{too_big:?}");
}
