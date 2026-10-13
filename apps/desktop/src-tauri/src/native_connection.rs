//! Native Messaging frames over a private local socket/pipe. No TCP port.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tower::ServiceExt;

use crate::bridge::Bridge;
use crate::native_registration as registration;

const MAX_INPUT: usize = 2 * 1024 * 1024;
const MAX_OUTPUT: usize = 900 * 1024; // Chrome accepts at most 1 MB from a host.
const MAX_CONNECTIONS: usize = 32;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
enum Message {
    Request {
        path: String,
        browser: String,
        #[serde(default)]
        origin: String,
        #[serde(default)]
        body: Option<Value>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default = "default_timeout", rename = "timeoutMs")]
        timeout_ms: u64,
    },
    Launch,
}
fn default_timeout() -> u64 {
    2500
}

#[derive(Debug, Serialize, Deserialize)]
struct Reply {
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}
impl Reply {
    fn error(error: &str) -> Self {
        Self {
            status: None,
            body: None,
            error: Some(error.into()),
        }
    }
}

pub struct Connection {
    pub data: PathBuf,
    bridge: Arc<Bridge>,
    running: AtomicBool,
    ready: AtomicBool,
    server_error: Mutex<Option<String>>,
    registration_error: Mutex<Option<String>>,
}
impl Connection {
    pub fn new(data: PathBuf, bridge: Arc<Bridge>) -> Arc<Self> {
        Arc::new(Self {
            data,
            bridge,
            running: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            server_error: Mutex::new(None),
            registration_error: Mutex::new(None),
        })
    }
    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub fn register_client(&self, id: &str) {
        if registration::valid_id(id)
            && !registration::allowed_origin(&self.data, &format!("chrome-extension://{id}/"))
        {
            let _ = self.register(Some(id));
        }
    }
    pub fn error(&self) -> Option<String> {
        self.server_error
            .lock()
            .ok()
            .and_then(|e| e.clone())
            .or_else(|| self.registration_error.lock().ok().and_then(|e| e.clone()))
    }
    pub fn register(&self, id: Option<&str>) -> io::Result<()> {
        let result = registration::register(&self.data, id);
        if let Ok(mut error) = self.registration_error.lock() {
            *error = result.as_ref().err().map(ToString::to_string);
        }
        result
    }
    pub fn start(self: &Arc<Self>) {
        if self
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let connection = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = connection.serve().await
                && let Ok(mut slot) = connection.server_error.lock()
            {
                *slot = Some(error.to_string());
            }
            connection.running.store(false, Ordering::Release);
            connection.ready.store(false, Ordering::Release);
        });
    }
    async fn serve(self: &Arc<Self>) -> io::Result<()> {
        let endpoint = endpoint(&self.data);
        let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
        #[cfg(unix)]
        let listener = {
            use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
            let parent = endpoint.parent().unwrap();
            match std::fs::create_dir(parent) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            let metadata = std::fs::symlink_metadata(parent)?;
            if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "The browser connection folder is not owned by this user",
                ));
            }
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
            match std::fs::symlink_metadata(&endpoint) {
                Ok(metadata)
                    if metadata.file_type().is_socket()
                        && metadata.uid() == unsafe { libc::geteuid() } =>
                {
                    std::fs::remove_file(&endpoint)?
                }
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "Unexpected browser connection file",
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let listener = tokio::net::UnixListener::bind(&endpoint)?;
            std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600))?;
            listener
        };
        #[cfg(windows)]
        let mut listener = tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&endpoint)?;
        if let Ok(mut slot) = self.server_error.lock() {
            *slot = None;
        }
        self.ready.store(true, Ordering::Release);
        loop {
            let permit = slots
                .clone()
                .acquire_owned()
                .await
                .map_err(io::Error::other)?;
            #[cfg(unix)]
            let (stream, _) = listener.accept().await?;
            #[cfg(windows)]
            let stream = {
                listener.connect().await?;
                let next = tokio::net::windows::named_pipe::ServerOptions::new()
                    .reject_remote_clients(true)
                    .create(&endpoint)?;
                std::mem::replace(&mut listener, next)
            };
            let bridge = self.bridge.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let _ =
                    tokio::time::timeout(Duration::from_secs(185), serve_request(stream, bridge))
                        .await;
            });
        }
    }
}

fn endpoint(data: &Path) -> PathBuf {
    let digest = Sha256::digest(data.to_string_lossy().as_bytes());
    let id: String = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    #[cfg(windows)]
    {
        PathBuf::from(format!(r"\\.\pipe\sdm-native-{id}"))
    }
    #[cfg(unix)]
    {
        Path::new("/tmp")
            .join(format!("sdm-native-{}-{id}", unsafe { libc::geteuid() }))
            .join("bridge.sock")
    }
}

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R, max: usize) -> io::Result<Vec<u8>> {
    let length = reader.read_u32_le().await? as usize;
    if length == 0 || length > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid native message size",
        ));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).await?;
    Ok(bytes)
}
async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, bytes: &[u8]) -> io::Result<()> {
    writer.write_u32_le(bytes.len() as u32).await?;
    writer.write_all(bytes).await?;
    writer.flush().await
}

async fn dispatch(message: Message, bridge: Arc<Bridge>) -> Reply {
    let Message::Request {
        path,
        browser,
        origin,
        body,
        headers,
        timeout_ms,
    } = message
    else {
        return Reply::error("invalidRequest");
    };
    if !matches!(
        path.as_str(),
        "/api/ping"
            | "/api/download"
            | "/api/media"
            | "/api/media/probe"
            | "/api/media/probe/start"
            | "/api/media/probe/status"
            | "/api/media/probe/cancel"
            | "/api/media/add"
            | "/api/links"
    ) || !matches!(
        browser.as_str(),
        "chrome" | "edge" | "chromium" | "firefox" | "brave" | "coccoc" | "opera" | "vivaldi"
    ) {
        return Reply::error("invalidRequest");
    }
    let mut request = Request::builder()
        .method(if path == "/api/ping" { "GET" } else { "POST" })
        .uri(path)
        .header("content-type", "application/json")
        .header("x-sdm-client", "extension")
        .header("x-sdm-browser", browser);
    if !origin.is_empty() {
        if origin.len() > 200
            || !(origin.starts_with("chrome-extension://")
                || origin.starts_with("moz-extension://"))
        {
            return Reply::error("invalidRequest");
        }
        request = request.header("origin", origin.trim_end_matches('/'));
    }
    // Only the media ownership header may be supplied by the client.
    if let Some(owner) = headers.get("X-SDM-Probe-Owner") {
        request = request.header("x-sdm-probe-owner", owner);
    }
    let body = body.map(|value| value.to_string()).unwrap_or_default();
    let Ok(request) = request.body(Body::from(body)) else {
        return Reply::error("invalidRequest");
    };
    let deadline = Duration::from_millis(timeout_ms.clamp(500, 180_000));
    match tokio::time::timeout(deadline, bridge.router().oneshot(request)).await {
        Ok(Ok(response)) => {
            let status = response.status().as_u16();
            match to_bytes(response.into_body(), MAX_OUTPUT).await {
                Ok(bytes) => Reply {
                    status: Some(status),
                    body: Some(String::from_utf8_lossy(&bytes).into_owned()),
                    error: None,
                },
                Err(_) => Reply::error("responseTooLarge"),
            }
        }
        Ok(Err(_)) => Reply::error("connectionError"),
        Err(_) => Reply::error("timeout"),
    }
}
async fn serve_request<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    bridge: Arc<Bridge>,
) -> io::Result<()> {
    let bytes = tokio::time::timeout(Duration::from_secs(3), read_frame(&mut stream, MAX_INPUT))
        .await
        .map_err(io::Error::other)??;
    let reply = match serde_json::from_slice::<Message>(&bytes) {
        Ok(message) => dispatch(message, bridge).await,
        Err(_) => Reply::error("invalidRequest"),
    };
    let mut bytes = serde_json::to_vec(&reply)?;
    if bytes.len() > MAX_OUTPUT {
        bytes = serde_json::to_vec(&Reply::error("responseTooLarge"))?;
    }
    write_frame(&mut stream, &bytes).await
}

async fn relay(bytes: &[u8], data: &Path) -> io::Result<Vec<u8>> {
    let endpoint = endpoint(data);
    #[cfg(unix)]
    let mut stream = tokio::net::UnixStream::connect(endpoint).await?;
    #[cfg(windows)]
    let mut stream = {
        let mut attempts = 0;
        loop {
            match tokio::net::windows::named_pipe::ClientOptions::new().open(&endpoint) {
                Ok(stream) => break stream,
                Err(error) if error.raw_os_error() == Some(231) && attempts < 10 => {
                    attempts += 1;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => return Err(error),
            }
        }
    };
    write_frame(&mut stream, bytes).await?;
    read_frame(&mut stream, MAX_OUTPUT).await
}

/// Run before Tauri/logging: stdout must contain only framed native messages.
pub fn run_mode() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "--register-browser-connection")
    {
        let result = registration::data_dir()
            .and_then(|data| registration::register(&data, args.get(1).map(String::as_str)));
        return Some(if result.is_ok() { 0 } else { 1 });
    }
    let chrome = args
        .first()
        .is_some_and(|arg| arg.starts_with("chrome-extension://"));
    let firefox = args
        .first()
        .is_some_and(|arg| arg == registration::FIREFOX_ID)
        || args
            .get(1)
            .is_some_and(|arg| arg == registration::FIREFOX_ID);
    if !chrome && !firefox {
        return None;
    }
    let result = (|| -> io::Result<()> {
        let data = registration::data_dir()?;
        let authorized = firefox || registration::allowed_origin(&data, &args[0]);
        let mut size = [0; 4];
        io::stdin().read_exact(&mut size)?;
        let length = u32::from_le_bytes(size) as usize;
        if length == 0 || length > MAX_INPUT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid native message size",
            ));
        }
        let mut bytes = vec![0; length];
        io::stdin().read_exact(&mut bytes)?;
        let reply = if !authorized {
            serde_json::to_vec(&Reply::error("notConfigured"))?
        } else {
            match serde_json::from_slice::<Message>(&bytes) {
                Ok(Message::Launch) => {
                    let mut command = std::process::Command::new(registration::executable()?);
                    command
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null());
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        command.creation_flags(0x08000000);
                    }
                    let reply = match command.spawn() {
                        Ok(child) => json!({"launched":true,"pid":child.id()}),
                        Err(_) => json!({"error":"launchFailed"}),
                    };
                    serde_json::to_vec(&reply)?
                }
                Ok(Message::Request { timeout_ms, .. }) => {
                    let mut value: Value = serde_json::from_slice(&bytes)?;
                    if chrome {
                        value["origin"] = json!(args[0].trim_end_matches('/'));
                    }
                    let bytes = serde_json::to_vec(&value)?;
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    match runtime.block_on(async {
                        tokio::time::timeout(
                            Duration::from_millis(timeout_ms.clamp(500, 180_000) + 1000),
                            relay(&bytes, &data),
                        )
                        .await
                    }) {
                        Ok(Ok(reply)) => reply,
                        Ok(Err(error))
                            if matches!(
                                error.kind(),
                                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                            ) =>
                        {
                            serde_json::to_vec(&Reply::error("appNotRunning"))?
                        }
                        _ => serde_json::to_vec(&Reply::error("connectionError"))?,
                    }
                }
                Err(_) => serde_json::to_vec(&Reply::error("invalidRequest"))?,
            }
        };
        if reply.len() > MAX_OUTPUT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Native response is too large",
            ));
        }
        let mut out = io::stdout().lock();
        out.write_all(&(reply.len() as u32).to_le_bytes())?;
        out.write_all(&reply)?;
        out.flush()
    })();
    Some(if result.is_ok() { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{BoxFuture, BridgeConfig, BrowserDownload, Handler, PageLinks};
    use crate::media::{MediaInfo, MediaSource};

    #[derive(Default)]
    struct Recorder(Arc<Mutex<Vec<BrowserDownload>>>);
    impl Handler for Recorder {
        fn config(&self) -> BridgeConfig {
            BridgeConfig {
                enabled: true,
                capture_extensions: vec!["exe".into()],
            }
        }
        fn download(&self, value: BrowserDownload) {
            self.0.lock().unwrap().push(value);
        }
        fn media(&self, _: MediaSource) {}
        fn probe_media(&self, _: MediaSource) -> BoxFuture<Result<MediaInfo, String>> {
            Box::pin(async { Err("Unsupported fixture".into()) })
        }
        fn add_media(&self, _: MediaSource, _: String) -> BoxFuture<Result<u64, String>> {
            Box::pin(async { Ok(1) })
        }
        fn links(&self, _: PageLinks) {}
    }
    fn request(path: &str, body: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({"action":"request","path":path,"browser":"chrome","body":body,"headers":{},"timeoutMs":2500})).unwrap()
    }
    #[tokio::test]
    async fn rejects_oversized_and_empty_frames_before_allocating() {
        for length in [0, MAX_INPUT as u32 + 1] {
            let (mut writer, mut reader) = tokio::io::duplex(8);
            writer.write_u32_le(length).await.unwrap();
            assert_eq!(
                read_frame(&mut reader, MAX_INPUT).await.unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }
    #[tokio::test]
    async fn restricts_routes_and_cannot_forward_arbitrary_request_headers() {
        let bridge = Bridge::new(Recorder::default());
        let forbidden =
            serde_json::from_slice(&request("https://example.com/", Value::Null)).unwrap();
        assert_eq!(
            dispatch(forbidden, bridge.clone()).await.error.as_deref(),
            Some("invalidRequest")
        );
        let allowed = serde_json::from_value(json!({"action":"request","path":"/api/ping","browser":"chrome","headers":{"Origin":"https://example.com","X-SDM-Client":"page"}})).unwrap();
        assert_eq!(dispatch(allowed, bridge).await.status, Some(200));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn private_socket_delivers_one_download_with_browser_metadata_without_tcp() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let recorder = Recorder::default();
        let downloads = recorder.0.clone();
        let connection = Connection::new(dir.path().to_owned(), Bridge::new(recorder));
        let server_connection = connection.clone();
        let server = tokio::spawn(async move { server_connection.serve().await });
        for _ in 0..100 {
            if connection.ready() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(connection.ready());
        let path = endpoint(dir.path());
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let bytes = request(
            "/api/download",
            json!({"url":"https://github.com/fixture/app.exe","fileName":"ứng dụng.exe","fileSize":8192,"cookies":"session=fixture","referrer":"https://github.com/fixture/releases","userAgent":"fixture"}),
        );
        let response: Reply =
            serde_json::from_slice(&relay(&bytes, dir.path()).await.unwrap()).unwrap();
        assert_eq!(response.status, Some(202));
        {
            let received = downloads.lock().unwrap();
            assert_eq!(received.len(), 1);
            assert_eq!(received[0].file_name.as_deref(), Some("ứng dụng.exe"));
            assert_eq!(received[0].file_size, Some(8192));
            assert_eq!(received[0].cookies.as_deref(), Some("session=fixture"));
        }
        let ping = request("/api/ping", Value::Null);
        let (first, second) = tokio::join!(relay(&ping, dir.path()), relay(&ping, dir.path()));
        assert_eq!(
            serde_json::from_slice::<Reply>(&first.unwrap())
                .unwrap()
                .status,
            Some(200)
        );
        assert_eq!(
            serde_json::from_slice::<Reply>(&second.unwrap())
                .unwrap()
                .status,
            Some(200)
        );
        server.abort();
        let _ = server.await;
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        assert!(relay(&bytes, dir.path()).await.is_err());
    }
}
