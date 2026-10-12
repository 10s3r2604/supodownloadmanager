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
    probes: AtomicUsize,
    if_ranges: std::sync::Mutex<Vec<String>>,
}

async fn start_server(data: Vec<u8>, failures: usize) -> (String, Arc<Server>) {
    let server = Arc::new(Server {
        data,
        failures_left: AtomicUsize::new(failures),
        data_requests: AtomicUsize::new(0),
        probes: AtomicUsize::new(0),
        if_ranges: std::sync::Mutex::new(Vec::new()),
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
        "file" | "named" | "flaky" | "changed" | "ifrangechanged" | "oscillating" | "weak"
        | "dated" | "missingvalidator" | "nowdated" => true,
        "lying" => is_probe,
        "norange" | "chunked" => false,
        _ => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .unwrap();
        }
    };
    let data_request = if !is_probe {
        if let Some(value) = headers.get(header::IF_RANGE).and_then(|v| v.to_str().ok()) {
            server.if_ranges.lock().unwrap().push(value.to_owned());
        }
        server.data_requests.fetch_add(1, Ordering::SeqCst) + 1
    } else {
        server.probes.fetch_add(1, Ordering::SeqCst);
        0
    };
    let changed = matches!(mode.as_str(), "changed" | "ifrangechanged")
        && (data_request > 1 || (is_probe && server.data_requests.load(Ordering::SeqCst) > 1));
    let version = if mode == "oscillating" {
        server.probes.load(Ordering::SeqCst) + usize::from(!is_probe)
    } else if changed {
        2
    } else {
        1
    };
    let etag = if mode == "weak" {
        format!("W/\"v{version}\"")
    } else {
        format!("\"v{version}\"")
    };
    let mut response = Response::builder();
    if mode != "dated" && !(mode == "missingvalidator" && !is_probe) {
        response = response.header(header::ETAG, &etag);
    }
    if matches!(mode.as_str(), "weak" | "dated") {
        response = response.header(header::LAST_MODIFIED, "Wed, 21 Oct 2015 07:28:00 GMT");
    }
    if mode == "nowdated" {
        // Some servers send the time of the request as the file's date.
        let second =
            server.probes.load(Ordering::SeqCst) + server.data_requests.load(Ordering::SeqCst);
        response = response.header(
            header::LAST_MODIFIED,
            format!("Wed, 21 Oct 2015 07:28:{:02} GMT", second % 60),
        );
    }
    if mode == "named" {
        response = response.header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"fallback.pdf\"; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf",
        );
    }

    let range = if mode == "ifrangechanged"
        && changed
        && headers
            .get(header::IF_RANGE)
            .is_some_and(|v| v != etag.as_str())
    {
        None
    } else {
        range
    };
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
    let body = if changed {
        Bytes::from(
            server.data[start as usize..end as usize]
                .iter()
                .map(|byte| byte ^ 0xff)
                .collect::<Vec<_>>(),
        )
    } else {
        Bytes::copy_from_slice(&server.data[start as usize..end as usize])
    };

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn custom_temp_preserves_pause_and_new_link_then_cleans() {
    let data = test_data(2 * 1024 * 1024);
    let (base, _) = start_server(data.clone(), 0).await;
    let output = tempfile::tempdir().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = output.path().join("data.bin");
    std::fs::write(temp.path().join("keep.txt"), b"unrelated").unwrap();
    let mut slow = request(
        format!("{base}/file/data.bin"),
        Destination::File(path.clone()),
    );
    slow.options.temp_dir = Some(temp.path().into());
    slow.options.speed_limit = Some(1024 * 1024);
    let engine = downloader();
    let handle = engine.start(slow);
    wait_for_bytes(&handle, 300 * 1024).await;
    handle.pause();
    let control = handle.control();
    assert!(matches!(
        handle.wait().await.unwrap(),
        Outcome::Paused {
            resumable: true,
            ..
        }
    ));
    let kept = control.progress().downloaded;
    let files = sdm_core::PartialFiles::new(&path, Some(temp.path()));
    assert!(files.data.exists() && files.state.exists());
    assert!(std::fs::read_dir(output.path()).unwrap().next().is_none());
    let fresh = format!("{base}/file/fresh.bin");
    let info = engine.probe(&fresh, &[]).await.unwrap();
    assert_eq!(
        sdm_core::retarget_partial_in(&path, Some(temp.path()), &info)
            .await
            .unwrap(),
        Some(kept)
    );
    let mut resume = request(fresh, Destination::File(path));
    resume.options.temp_dir = Some(temp.path().into());
    let handle = engine.start(resume);
    let control = handle.control();
    let (done, _) = completed(handle).await;
    assert_eq!(control.progress().resumed_from, kept);
    assert_eq!(std::fs::read(done).unwrap(), data);
    assert!(!files.data.exists() && !files.state.exists());
    assert!(!files.directory().unwrap().exists());
    assert_eq!(
        std::fs::read(temp.path().join("keep.txt")).unwrap(),
        b"unrelated"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn custom_temp_migrates_legacy_progress_without_redownloading() {
    check_legacy_migration(None).await;
}

async fn check_legacy_migration(temp_parent: Option<&Path>) {
    let data = test_data(2 * 1024 * 1024);
    let (base, _) = start_server(data.clone(), 0).await;
    let output = tempfile::tempdir().unwrap();
    let temp = match temp_parent {
        Some(parent) => tempfile::tempdir_in(parent).unwrap(),
        None => tempfile::tempdir().unwrap(),
    };
    let path = output.path().join("legacy.bin");
    let mut slow = request(
        format!("{base}/file/legacy.bin"),
        Destination::File(path.clone()),
    );
    slow.options.speed_limit = Some(1024 * 1024);
    let engine = downloader();
    let handle = engine.start(slow.clone());
    wait_for_bytes(&handle, 300 * 1024).await;
    handle.pause();
    let control = handle.control();
    assert!(matches!(
        handle.wait().await.unwrap(),
        Outcome::Paused { .. }
    ));
    let kept = control.progress().downloaded;
    assert!(sdm_core::partial_state_path(&path).exists());
    slow.options.temp_dir = Some(temp.path().into());
    slow.options.speed_limit = None;
    let handle = engine.start(slow);
    let control = handle.control();
    let (done, _) = completed(handle).await;
    assert_eq!(control.progress().resumed_from, kept);
    assert_eq!(std::fs::read(done).unwrap(), data);
    assert!(leftovers(output.path()).is_empty());
    assert!(std::fs::read_dir(temp.path()).unwrap().next().is_none());
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn moves_legacy_and_completed_data_between_real_filesystems() {
    use std::os::unix::fs::MetadataExt;
    assert_ne!(
        std::fs::metadata(std::env::temp_dir()).unwrap().dev(),
        std::fs::metadata("/dev/shm").unwrap().dev()
    );
    check_legacy_migration(Some(Path::new("/dev/shm"))).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn custom_temp_isolates_equal_names_and_cancel_removes_only_its_data() {
    let data = test_data(2 * 1024 * 1024);
    let (base, _) = start_server(data.clone(), 0).await;
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let engine = downloader();
    let mut a = request(
        format!("{base}/file/same.bin"),
        Destination::File(first.path().join("same.bin")),
    );
    a.options.temp_dir = Some(temp.path().into());
    a.options.speed_limit = Some(1024 * 1024);
    let mut b = a.clone();
    b.destination = Destination::File(second.path().join("same.bin"));
    let a = engine.start(a);
    let b = engine.start(b);
    wait_for_bytes(&a, 100 * 1024).await;
    wait_for_bytes(&b, 100 * 1024).await;
    let a_files = sdm_core::PartialFiles::new(&first.path().join("same.bin"), Some(temp.path()));
    let b_files = sdm_core::PartialFiles::new(&second.path().join("same.bin"), Some(temp.path()));
    assert_ne!(a_files.data, b_files.data);
    assert!(a_files.data.exists() && b_files.data.exists());
    a.cancel();
    assert_eq!(a.wait().await.unwrap(), Outcome::Cancelled);
    assert!(!a_files.directory().unwrap().exists());
    let (path, _) = completed(b).await;
    assert_eq!(std::fs::read(path).unwrap(), data);
    assert!(std::fs::read_dir(temp.path()).unwrap().next().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_final_move_keeps_all_bytes_for_retry() {
    let data = test_data(1024 * 1024);
    let (base, server) = start_server(data.clone(), 0).await;
    let output = tempfile::tempdir().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = output.path().join("blocked.bin");
    let mut download = request(
        format!("{base}/file/blocked.bin"),
        Destination::File(path.clone()),
    );
    download.options.temp_dir = Some(temp.path().into());
    download.options.speed_limit = Some(1024 * 1024);
    let engine = downloader();
    let handle = engine.start(download.clone());
    wait_for_bytes(&handle, 100 * 1024).await;
    std::fs::create_dir(&path).unwrap();
    assert!(handle.wait().await.is_err());
    let files = sdm_core::PartialFiles::new(&path, Some(temp.path()));
    assert!(files.data.exists() && files.state.exists());
    let before = server.data_requests.load(Ordering::SeqCst);
    std::fs::remove_dir(&path).unwrap();
    download.options.speed_limit = None;
    let handle = engine.start(download);
    let control = handle.control();
    let (done, _) = completed(handle).await;
    assert_eq!(control.progress().resumed_from, data.len() as u64);
    assert_eq!(server.data_requests.load(Ordering::SeqCst), before);
    assert_eq!(std::fs::read(done).unwrap(), data);
    assert!(!files.directory().unwrap().exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restarts_all_ranges_when_same_size_representation_changes() {
    for mode in ["changed", "ifrangechanged"] {
        let data = test_data(1024 * 1024);
        let expected: Vec<_> = data.iter().map(|byte| byte ^ 0xff).collect();
        let (base, server) = start_server(data, 0).await;
        let output = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = output.path().join("changed.bin");
        let mut download = request(
            format!("{base}/{mode}/changed.bin"),
            Destination::File(path.clone()),
        );
        download.options.temp_dir = Some(temp.path().into());
        let handle = downloader().start(download);
        let control = handle.control();
        let (done, _) = completed(handle).await;
        assert_eq!(
            std::fs::read(done).unwrap(),
            expected,
            "mixed server versions in {mode}"
        );
        assert_eq!(server.probes.load(Ordering::SeqCst), 2);
        assert_eq!(control.progress().resumed_from, 0);
        let if_ranges = server.if_ranges.lock().unwrap();
        assert!(if_ranges.iter().any(|v| v == "\"v1\""));
        assert!(if_ranges.iter().any(|v| v == "\"v2\""));
        assert!(std::fs::read_dir(temp.path()).unwrap().next().is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unstable_validators_fail_after_one_clean_restart() {
    for mode in ["oscillating"] {
        let (base, server) = start_server(test_data(256 * 1024), 0).await;
        let output = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = output.path().join("unstable.bin");
        let mut download = request(
            format!("{base}/{mode}/unstable.bin"),
            Destination::File(path.clone()),
        );
        download.options.temp_dir = Some(temp.path().into());
        let handle = downloader().start(download);
        let control = handle.control();
        let outcome = tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .unwrap();
        assert!(matches!(outcome, Err(Error::RemoteChanged)), "{outcome:?}");
        assert_eq!(server.probes.load(Ordering::SeqCst), 2);
        assert_eq!(control.progress().downloaded, 0);
        assert!(!path.exists());
        assert!(std::fs::read_dir(temp.path()).unwrap().next().is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tolerates_missing_tags_and_changing_dates_on_ranges() {
    // Some servers leave the entity tag out of range responses, or send a
    // new date with every response, for a file that does not change.
    for mode in ["missingvalidator", "nowdated"] {
        let data = test_data(1024 * 1024);
        let (base, _server) = start_server(data.clone(), 0).await;
        let output = tempfile::tempdir().unwrap();
        let (path, _) = completed(downloader().start(request(
            format!("{base}/{mode}/file.bin"),
            Destination::Directory(output.path().into()),
        )))
        .await;
        assert_eq!(std::fs::read(path).unwrap(), data, "{mode}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn if_range_uses_date_instead_of_weak_etag() {
    for mode in ["weak", "dated"] {
        let data = test_data(256 * 1024);
        let (base, server) = start_server(data.clone(), 0).await;
        let output = tempfile::tempdir().unwrap();
        let (path, _) = completed(downloader().start(request(
            format!("{base}/{mode}/file.bin"),
            Destination::Directory(output.path().into()),
        )))
        .await;
        assert_eq!(std::fs::read(path).unwrap(), data);
        let seen = server.if_ranges.lock().unwrap();
        assert!(!seen.is_empty());
        assert!(
            seen.iter()
                .all(|value| value == "Wed, 21 Oct 2015 07:28:00 GMT")
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn corrupt_layout_or_partial_length_never_reports_completed_progress() {
    let data = test_data(128 * 1024);
    let size = data.len() as u64;
    let (base, server) = start_server(data.clone(), 0).await;
    let url = format!("{base}/file/file.bin");
    let valid = vec![serde_json::json!({"start":0,"pos":size,"end":size})];
    let cases = [
        (Some(size / 2), valid.clone()),
        (Some(0), valid.clone()),
        (None, valid.clone()),
        (Some(size + 1), valid.clone()),
        (
            Some(size),
            vec![
                serde_json::json!({"start":0,"pos":size,"end":size}),
                serde_json::json!({"start":0,"pos":size,"end":size}),
            ],
        ),
        (
            Some(size),
            vec![
                serde_json::json!({"start":0,"pos":size / 2,"end":size / 2}),
                serde_json::json!({"start":size / 2 + 1,"pos":size,"end":size}),
            ],
        ),
    ];
    for (length, segments) in cases {
        let output = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = output.path().join("file.bin");
        let files = sdm_core::PartialFiles::new(&path, Some(temp.path()));
        std::fs::create_dir_all(files.directory().unwrap()).unwrap();
        if let Some(length) = length {
            std::fs::File::create(&files.data)
                .unwrap()
                .set_len(length)
                .unwrap();
        }
        let saved = serde_json::json!({"version":1,"url":url,"size":size,"etag":"\"v1\"","last_modified":null,"segments":segments});
        std::fs::write(&files.state, serde_json::to_vec(&saved).unwrap()).unwrap();
        let before = server.data_requests.load(Ordering::SeqCst);
        let mut download = request(url.clone(), Destination::File(path));
        download.options.temp_dir = Some(temp.path().into());
        let handle = downloader().start(download);
        let control = handle.control();
        let (done, bytes) = completed(handle).await;
        assert_eq!(bytes, size);
        assert_eq!(std::fs::read(done).unwrap(), data);
        assert_eq!(control.progress().resumed_from, 0);
        assert!(server.data_requests.load(Ordering::SeqCst) > before);
        assert!(!files.directory().unwrap().exists());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pausing_while_finishing_keeps_data_that_cannot_resume() {
    // Without ranges nothing can be kept for later, so a pause (or quitting
    // the app) during the last checks must let the file finish.
    let data = test_data(64 * 1024 * 1024);
    let (base, _) = start_server(data.clone(), 0).await;
    let output = tempfile::tempdir().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = output.path().join("whole.bin");
    let mut download = request(
        format!("{base}/norange/whole.bin"),
        Destination::File(path.clone()),
    );
    download.options.temp_dir = Some(temp.path().into());
    download.options.completion_id = Some("row".into());
    let handle = downloader().start(download);
    let control = handle.control();
    let deadline = Instant::now() + Duration::from_secs(30);
    while control.progress().phase != sdm_core::Phase::Finishing {
        assert!(
            Instant::now() < deadline,
            "never reached the finishing phase"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    control.pause();
    let (done, size) = completed(handle).await;
    assert_eq!(size, data.len() as u64);
    assert_eq!(std::fs::read(done).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn completed_receipt_survives_core_cleanup_until_history_acknowledgement() {
    let data = test_data(256 * 1024);
    let (base, _) = start_server(data.clone(), 0).await;
    let output = tempfile::tempdir().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = output.path().join("receipt.bin");
    let mut download = request(
        format!("{base}/file/receipt.bin"),
        Destination::File(path.clone()),
    );
    download.options.temp_dir = Some(temp.path().into());
    download.options.completion_id = Some("history-row".into());
    let (done, size) = completed(downloader().start(download)).await;
    let files = sdm_core::PartialFiles::new(&path, Some(temp.path()));
    assert!(!files.data.exists() && !files.state.exists());
    assert!(files.completion.exists());
    let receipt = sdm_core::reconcile_completion(&path, Some(temp.path()), "history-row")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.path, done);
    assert_eq!(receipt.size, size);
    assert_eq!(
        sdm_core::reconcile_completion(&path, Some(temp.path()), "another-row")
            .await
            .unwrap(),
        None
    );
    sdm_core::acknowledge_completion(&path, Some(temp.path()), "history-row")
        .await
        .unwrap();
    assert!(!files.directory().unwrap().exists());
    assert_eq!(std::fs::read(path).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changing_url_preserves_partial_until_fresh_link_is_validated() {
    let data = test_data(128 * 1024);
    let size = data.len() as u64;
    let (base, _) = start_server(data.clone(), 0).await;
    let output = tempfile::tempdir().unwrap();
    let path = output.path().join("file.bin");
    let files = sdm_core::PartialFiles::new(&path, None);
    std::fs::write(&files.data, &data).unwrap();
    let saved = serde_json::json!({"version":1,"url":format!("{base}/file/expired.bin"),"size":size,"etag":"\"v1\"","last_modified":null,"segments":[{"start":0,"pos":size / 2,"end":size}]});
    let original_state = serde_json::to_vec(&saved).unwrap();
    std::fs::write(&files.state, &original_state).unwrap();
    let fresh_url = format!("{base}/file/fresh.bin");
    let engine = downloader();
    let outcome = engine
        .start(request(fresh_url.clone(), Destination::File(path.clone())))
        .wait()
        .await;
    assert!(matches!(outcome, Err(Error::NotSameFile(_))));
    assert_eq!(std::fs::read(&files.state).unwrap(), original_state);
    assert_eq!(std::fs::read(&files.data).unwrap(), data);
    let info = engine.probe(&fresh_url, &[]).await.unwrap();
    assert_eq!(
        sdm_core::retarget_partial(&path, &info).await.unwrap(),
        Some(size / 2)
    );
    let handle = engine.start(request(fresh_url, Destination::File(path.clone())));
    let control = handle.control();
    completed(handle).await;
    assert_eq!(control.progress().resumed_from, size / 2);
    assert_eq!(std::fs::read(path).unwrap(), data);
}
