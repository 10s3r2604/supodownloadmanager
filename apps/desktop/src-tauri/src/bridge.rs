//! A small HTTP server on `127.0.0.1` that the browser extension talks to.
//!
//! Only browser extensions may use it. Every request must carry the
//! `X-SDM-Client` header: a web page can only send such a header after a CORS
//! preflight, which this server never approves. Requests whose `Origin` is a
//! web page are rejected as well.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::media::{MediaInfo, MediaSource};

pub type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;

pub const CLIENT_HEADER: &str = "x-sdm-client";
pub const BROWSER_HEADER: &str = "x-sdm-browser";

const EXTENSION_ORIGINS: &[&str] = &[
    "chrome-extension://",
    "moz-extension://",
    "extension://",
    "safari-web-extension://",
];

/// A download handed over by the browser.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDownload {
    pub url: String,
    #[serde(default)]
    pub referrer: Option<String>,
    #[serde(default)]
    pub cookies: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub file_size: Option<u64>,
}

impl BrowserDownload {
    /// Request headers that make the server treat SDM like the browser.
    pub fn headers(&self) -> Vec<(String, String)> {
        [
            ("Referer", &self.referrer),
            ("Cookie", &self.cookies),
            ("User-Agent", &self.user_agent),
        ]
        .into_iter()
        .filter_map(|(name, value)| {
            let value = value.as_deref()?.trim();
            (!value.is_empty()).then(|| (name.to_owned(), value.to_owned()))
        })
        .collect()
    }
}

/// The links of a web page, for "download all links".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageLinks {
    pub page_url: String,
    #[serde(default)]
    pub title: Option<String>,
    pub links: Vec<PageLink>,
    /// Cookies of the page's site; only sent to links on that site.
    #[serde(default)]
    pub cookies: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageLink {
    pub url: String,
    /// The link's text or the image's description.
    #[serde(default)]
    pub text: Option<String>,
}

/// The most links one page may send.
pub const MAX_PAGE_LINKS: usize = 5000;

/// What the extension needs to know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeConfig {
    pub enabled: bool,
    pub capture_extensions: Vec<String>,
}

/// What the app does with requests from the extension.
pub trait Handler: Send + Sync + 'static {
    fn native_client(&self, _id: &str) {}
    fn native_ready(&self) -> bool {
        false
    }
    fn config(&self) -> BridgeConfig;
    fn download(&self, download: BrowserDownload);
    /// A video on a web page, to download with yt-dlp.
    fn media(&self, source: MediaSource);
    /// The qualities of a video page (read once, then remembered a while).
    fn probe_media(&self, source: MediaSource) -> BoxFuture<Result<MediaInfo, String>>;
    /// A cancellable browser interest; dropping it must release only that interest.
    fn probe_media_browser(&self, source: MediaSource) -> BoxFuture<Result<MediaInfo, String>> {
        self.probe_media(source)
    }
    /// Starts downloading a video in one of the qualities found.
    fn add_media(&self, source: MediaSource, option: String) -> BoxFuture<Result<u64, String>>;
    /// The links of a page, for the user to pick from.
    fn links(&self, links: PageLinks);
}

/// Asks for the qualities of a video page.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProbeRequest {
    #[serde(flatten)]
    source: MediaSource,
    /// `false` only starts reading the page, so a later request is quick.
    #[serde(default)]
    wait: bool,
}

/// Downloads a video in the quality picked in the page's menu.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddMediaRequest {
    source: MediaSource,
    option: String,
}

pub struct Bridge {
    handler: Box<dyn Handler>,
    seen: Mutex<HashMap<String, Instant>>,
    probes: Mutex<HashMap<String, Arc<BrowserProbe>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Ping {
    app: &'static str,
    native_messaging: bool,
    version: &'static str,
    /// The app downloads videos from web pages (`/api/media`).
    media: bool,
    media_probe_jobs: bool,
    /// The app takes magnet links in `/api/download`.
    torrent: bool,
    /// The app takes the links of a page (`/api/links`).
    links: bool,
    #[serde(flatten)]
    config: BridgeConfig,
}

impl Bridge {
    pub fn new(handler: impl Handler) -> Arc<Self> {
        Arc::new(Self {
            handler: Box::new(handler),
            seen: Mutex::new(HashMap::new()),
            probes: Mutex::new(HashMap::new()),
        })
    }

    /// Browsers whose extension contacted us, with seconds since then.
    pub fn connected(&self) -> HashMap<String, u64> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(browser, at)| (browser.clone(), at.elapsed().as_secs()))
            .collect()
    }

    pub fn router(self: Arc<Self>) -> Router {
        Router::new()
            .route("/api/ping", get(ping))
            .route("/api/download", post(download))
            .route("/api/media", post(media))
            .route("/api/media/probe", post(media_probe))
            .route("/api/media/probe/start", post(media_probe_start))
            .route("/api/media/probe/status", post(media_probe_status))
            .route("/api/media/probe/cancel", post(media_probe_cancel))
            .route("/api/media/add", post(media_add))
            .route("/api/links", post(links))
            .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
            .with_state(self)
    }

    /// Serves until the app quits. Fails when the port is taken.
    pub async fn serve(self: Arc<Self>, port: u16) -> std::io::Result<()> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
        tracing::info!(port, "browser bridge listening");
        axum::serve(listener, self.router()).await
    }

    fn remember(&self, headers: &HeaderMap) {
        let browser: String = headers
            .get(BROWSER_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(20)
            .collect::<String>()
            .to_ascii_lowercase();
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(browser, Instant::now());
    }
}

fn from_extension(headers: &HeaderMap) -> bool {
    if !headers.contains_key(CLIENT_HEADER) {
        return false;
    }
    match headers.get(header::ORIGIN) {
        None => true,
        Some(origin) => {
            let origin = origin.to_str().unwrap_or_default();
            EXTENSION_ORIGINS
                .iter()
                .any(|prefix| origin.starts_with(prefix))
        }
    }
}

async fn ping(State(bridge): State<Arc<Bridge>>, headers: HeaderMap) -> Response {
    if !from_extension(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    bridge.remember(&headers);
    if let Some(id) = headers
        .get("x-sdm-extension-id")
        .and_then(|value| value.to_str().ok())
    {
        let expected = format!("chrome-extension://{id}");
        let origin_matches = headers.get(header::ORIGIN).is_none_or(|origin| {
            origin
                .to_str()
                .is_ok_and(|origin| origin.trim_end_matches('/') == expected)
        });
        if crate::native_registration::valid_id(id) && origin_matches {
            bridge.handler.native_client(id);
        }
    }
    Json(Ping {
        app: "sdm",
        native_messaging: bridge.handler.native_ready(),
        version: env!("CARGO_PKG_VERSION"),
        media: true,
        media_probe_jobs: true,
        torrent: true,
        links: true,
        config: bridge.handler.config(),
    })
    .into_response()
}

async fn download(
    State(bridge): State<Arc<Bridge>>,
    headers: HeaderMap,
    Json(request): Json<BrowserDownload>,
) -> Response {
    if !from_extension(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    bridge.remember(&headers);
    if !bridge.handler.config().enabled {
        return (StatusCode::CONFLICT, "browser integration is turned off").into_response();
    }
    let valid = crate::torrent::is_magnet(&request.url)
        || url::Url::parse(&request.url).is_ok_and(|url| matches!(url.scheme(), "http" | "https"));
    if !valid {
        return (
            StatusCode::BAD_REQUEST,
            "only http, https and magnet links are supported",
        )
            .into_response();
    }
    bridge.handler.download(request);
    StatusCode::ACCEPTED.into_response()
}

/// Checks a request about a video page; `None` when it may go ahead.
fn refuse_media(bridge: &Bridge, headers: &HeaderMap, page_url: &str) -> Option<Response> {
    if !from_extension(headers) {
        return Some(StatusCode::FORBIDDEN.into_response());
    }
    bridge.remember(headers);
    if !bridge.handler.config().enabled {
        return Some((StatusCode::CONFLICT, "browser integration is turned off").into_response());
    }
    let valid = url::Url::parse(page_url).is_ok_and(|url| matches!(url.scheme(), "http" | "https"));
    (!valid).then(|| {
        (
            StatusCode::BAD_REQUEST,
            "only http and https pages are supported",
        )
            .into_response()
    })
}

async fn media(
    State(bridge): State<Arc<Bridge>>,
    headers: HeaderMap,
    Json(source): Json<MediaSource>,
) -> Response {
    if let Some(refused) = refuse_media(&bridge, &headers, &source.page_url) {
        return refused;
    }
    bridge.handler.media(source);
    StatusCode::ACCEPTED.into_response()
}

async fn media_probe(
    State(bridge): State<Arc<Bridge>>,
    headers: HeaderMap,
    Json(request): Json<ProbeRequest>,
) -> Response {
    if let Some(refused) = refuse_media(&bridge, &headers, &request.source.page_url) {
        return refused;
    }
    let probe = bridge.handler.probe_media_browser(request.source);
    if !request.wait {
        tokio::spawn(async move {
            let _ = probe.await;
        });
        return StatusCode::ACCEPTED.into_response();
    }
    match probe.await {
        Ok(info) => Json(info).into_response(),
        Err(error) => (StatusCode::UNPROCESSABLE_ENTITY, error).into_response(),
    }
}

/// A client ID is an opaque capability held by one content-script document.
/// Origin and owner headers isolate tabs, frames, profiles and extensions.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProbeClient {
    client_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartProbeRequest {
    source: MediaSource,
    client_id: String,
}

struct BrowserProbe {
    fingerprint: Option<u64>,
    touched: Mutex<tokio::time::Instant>,
    result: Mutex<Option<Result<MediaInfo, String>>>,
    cancel: tokio::sync::watch::Sender<bool>,
}

const BROWSER_PROBE_LEASE: Duration = Duration::from_secs(20);
const BROWSER_PROBE_TIMEOUT: Duration = Duration::from_secs(300);
const BROWSER_PROBE_KEEP: Duration = Duration::from_secs(60);
const BROWSER_PROBE_LIMIT: usize = 128;

fn probe_client_key(
    bridge: &Bridge,
    headers: &HeaderMap,
    client_id: &str,
) -> Result<String, (StatusCode, &'static str)> {
    if !from_extension(headers) {
        return Err((StatusCode::FORBIDDEN, "video probe forbidden"));
    }
    bridge.remember(headers);
    if !bridge.handler.config().enabled {
        return Err((StatusCode::CONFLICT, "browser integration is turned off"));
    }
    let owner = headers
        .get("x-sdm-probe-owner")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if owner.is_empty()
        || owner.len() > 200
        || client_id.is_empty()
        || client_id.len() > 100
        || !client_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err((StatusCode::BAD_REQUEST, "invalid video probe client"));
    }
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let origin = origin.trim_end_matches('/');
    let owner_id = owner.split(':').next().unwrap_or_default();
    let stable_origin;
    let origin = if crate::native_registration::valid_id(owner_id) {
        stable_origin = format!("chrome-extension://{owner_id}");
        if !origin.is_empty() && origin != stable_origin {
            return Err((StatusCode::BAD_REQUEST, "video probe origin mismatch"));
        }
        stable_origin.as_str()
    } else {
        origin
    };
    let browser = headers
        .get(BROWSER_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    Ok(format!("{origin}|{browser}|{owner}|{client_id}"))
}

fn prune_browser_probes(probes: &mut HashMap<String, Arc<BrowserProbe>>) {
    probes.retain(|_, job| {
        let finished = job
            .result
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        let keep = if finished {
            BROWSER_PROBE_KEEP
        } else {
            BROWSER_PROBE_LEASE
        };
        if job
            .touched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elapsed()
            > keep
        {
            job.cancel.send_replace(true);
            false
        } else {
            true
        }
    });
}

fn browser_probe_response(job: &BrowserProbe) -> Response {
    match job
        .result
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        None => Json(serde_json::json!({ "state": "pending" })).into_response(),
        Some(Ok(info)) => {
            Json(serde_json::json!({ "state": "done", "info": info })).into_response()
        }
        Some(Err(error)) => {
            Json(serde_json::json!({ "state": "error", "error": error })).into_response()
        }
    }
}

async fn media_probe_start(
    State(bridge): State<Arc<Bridge>>,
    headers: HeaderMap,
    Json(request): Json<StartProbeRequest>,
) -> Response {
    if let Some(refused) = refuse_media(&bridge, &headers, &request.source.page_url) {
        return refused;
    }
    let key = match probe_client_key(&bridge, &headers, &request.client_id) {
        Ok(key) => key,
        Err(response) => return response.into_response(),
    };
    let fingerprint = crate::media::source_key(&request.source, "");
    let job = {
        let mut jobs = bridge.probes.lock().unwrap_or_else(PoisonError::into_inner);
        prune_browser_probes(&mut jobs);
        if let Some(job) = jobs.get(&key) {
            if job
                .fingerprint
                .is_some_and(|existing| existing != fingerprint)
            {
                return (StatusCode::CONFLICT, "video probe client already used").into_response();
            }
            *job.touched.lock().unwrap_or_else(PoisonError::into_inner) =
                tokio::time::Instant::now();
            return browser_probe_response(job);
        }
        if jobs.len() >= BROWSER_PROBE_LIMIT {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "video discovery is busy; try again shortly",
            )
                .into_response();
        }
        let job = Arc::new(BrowserProbe {
            fingerprint: Some(fingerprint),
            touched: Mutex::new(tokio::time::Instant::now()),
            result: Mutex::new(None),
            cancel: tokio::sync::watch::channel(false).0,
        });
        jobs.insert(key, job.clone());
        job
    };
    let probe = bridge.handler.probe_media_browser(request.source);
    let running = job.clone();
    tokio::spawn(async move {
        let mut cancelled = running.cancel.subscribe();
        let abandoned = async {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                if running
                    .touched
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .elapsed()
                    > BROWSER_PROBE_LEASE
                {
                    break;
                }
            }
        };
        let result = tokio::select! {
            biased;
            _ = crate::media::wait_cancelled(&mut cancelled) => Err("video discovery cancelled".into()),
            _ = abandoned => Err("video discovery expired; try again".into()),
            _ = tokio::time::sleep(BROWSER_PROBE_TIMEOUT) => Err("the site took too long to answer; try again".into()),
            result = probe => result,
        };
        *running
            .result
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(result);
        *running
            .touched
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = tokio::time::Instant::now();
    });
    browser_probe_response(&job)
}

async fn media_probe_status(
    State(bridge): State<Arc<Bridge>>,
    headers: HeaderMap,
    Json(request): Json<ProbeClient>,
) -> Response {
    let key = match probe_client_key(&bridge, &headers, &request.client_id) {
        Ok(key) => key,
        Err(response) => return response.into_response(),
    };
    let mut jobs = bridge.probes.lock().unwrap_or_else(PoisonError::into_inner);
    prune_browser_probes(&mut jobs);
    let Some(job) = jobs.get(&key) else {
        return (StatusCode::NOT_FOUND, "video discovery expired; try again").into_response();
    };
    *job.touched.lock().unwrap_or_else(PoisonError::into_inner) = tokio::time::Instant::now();
    browser_probe_response(job)
}

async fn media_probe_cancel(
    State(bridge): State<Arc<Bridge>>,
    headers: HeaderMap,
    Json(request): Json<ProbeClient>,
) -> Response {
    let key = match probe_client_key(&bridge, &headers, &request.client_id) {
        Ok(key) => key,
        Err(response) => return response.into_response(),
    };
    let mut jobs = bridge.probes.lock().unwrap_or_else(PoisonError::into_inner);
    prune_browser_probes(&mut jobs);
    // Keep a short tombstone: close may arrive while /start is collecting cookies.
    if let Some(job) = jobs.get(&key) {
        job.cancel.send_replace(true);
        *job.result.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(Err("video discovery cancelled".into()));
    } else if jobs.len() < BROWSER_PROBE_LIMIT {
        jobs.insert(
            key,
            Arc::new(BrowserProbe {
                fingerprint: None,
                touched: Mutex::new(tokio::time::Instant::now()),
                result: Mutex::new(Some(Err("video discovery cancelled".into()))),
                cancel: tokio::sync::watch::channel(true).0,
            }),
        );
    }
    StatusCode::ACCEPTED.into_response()
}

async fn media_add(
    State(bridge): State<Arc<Bridge>>,
    headers: HeaderMap,
    Json(request): Json<AddMediaRequest>,
) -> Response {
    if let Some(refused) = refuse_media(&bridge, &headers, &request.source.page_url) {
        return refused;
    }
    match bridge
        .handler
        .add_media(request.source, request.option)
        .await
    {
        Ok(id) => Json(serde_json::json!({ "id": id })).into_response(),
        Err(error) => (StatusCode::UNPROCESSABLE_ENTITY, error).into_response(),
    }
}

async fn links(
    State(bridge): State<Arc<Bridge>>,
    headers: HeaderMap,
    Json(mut request): Json<PageLinks>,
) -> Response {
    if let Some(refused) = refuse_media(&bridge, &headers, &request.page_url) {
        return refused;
    }
    request.links.retain(|link| {
        url::Url::parse(&link.url).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
    });
    request.links.truncate(MAX_PAGE_LINKS);
    if request.links.is_empty() {
        return (StatusCode::BAD_REQUEST, "no links to download").into_response();
    }
    bridge.handler.links(request);
    StatusCode::ACCEPTED.into_response()
}

#[cfg(test)]
mod tests {
    #[test]
    fn browser_probe_identity_survives_transport_changes_and_rejects_a_different_origin() {
        let bridge = Bridge::new(Recorder {
            enabled: true,
            ..Default::default()
        });
        let id = "abcdefghijklmnopabcdefghijklmnop";
        let mut headers = HeaderMap::new();
        headers.insert(CLIENT_HEADER, "extension".parse().unwrap());
        headers.insert(BROWSER_HEADER, "chrome".parse().unwrap());
        headers.insert(
            "x-sdm-probe-owner",
            format!("{id}:12:0:document-a").parse().unwrap(),
        );
        let native = probe_client_key(&bridge, &headers, "stable-client").unwrap();
        headers.insert(
            header::ORIGIN,
            format!("chrome-extension://{id}/").parse().unwrap(),
        );
        assert_eq!(
            probe_client_key(&bridge, &headers, "stable-client").unwrap(),
            native
        );
        headers.insert(
            header::ORIGIN,
            "chrome-extension://ponmlkjihgfedcbaponmlkjihgfedcba"
                .parse()
                .unwrap(),
        );
        assert!(probe_client_key(&bridge, &headers, "stable-client").is_err());
    }
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;

    #[derive(Default)]
    struct Recorder {
        enabled: bool,
        received: Arc<Mutex<Vec<BrowserDownload>>>,
        videos: Arc<Mutex<Vec<MediaSource>>>,
        pages: Arc<Mutex<Vec<PageLinks>>>,
    }

    impl Handler for Recorder {
        fn config(&self) -> BridgeConfig {
            BridgeConfig {
                enabled: self.enabled,
                capture_extensions: vec!["zip".into()],
            }
        }

        fn download(&self, download: BrowserDownload) {
            self.received.lock().unwrap().push(download);
        }

        fn media(&self, source: MediaSource) {
            self.videos.lock().unwrap().push(source);
        }

        fn probe_media(&self, source: MediaSource) -> BoxFuture<Result<MediaInfo, String>> {
            Box::pin(async move {
                if source.page_url.contains("slow") {
                    tokio::time::sleep(Duration::from_secs(45)).await;
                }
                if source.page_url.contains("never") {
                    std::future::pending::<()>().await;
                }
                if source.page_url.contains("broken") {
                    return Err("Unsupported URL".into());
                }
                Ok(crate::media::MediaInfo {
                    title: "Clip".into(),
                    thumbnail: None,
                    duration: Some(60.0),
                    site: "Example".into(),
                    url: source.page_url,
                    options: Vec::new(),
                    default: None,
                })
            })
        }

        fn add_media(
            &self,
            _source: MediaSource,
            option: String,
        ) -> BoxFuture<Result<u64, String>> {
            Box::pin(async move {
                if option == "v720" {
                    Ok(7)
                } else {
                    Err("no such quality".into())
                }
            })
        }

        fn links(&self, links: PageLinks) {
            self.pages.lock().unwrap().push(links);
        }
    }

    fn bridge(enabled: bool) -> (Arc<Bridge>, Arc<Mutex<Vec<BrowserDownload>>>) {
        let received = Arc::new(Mutex::new(Vec::new()));
        let bridge = Bridge::new(Recorder {
            enabled,
            received: received.clone(),
            videos: Arc::default(),
            pages: Arc::default(),
        });
        (bridge, received)
    }

    #[tokio::test]
    async fn takes_the_links_of_a_page() {
        let pages = Arc::new(Mutex::new(Vec::new()));
        let bridge = Bridge::new(Recorder {
            enabled: true,
            pages: pages.clone(),
            ..Recorder::default()
        });
        let post = |body: &str| {
            Request::post("/api/links")
                .header("content-type", "application/json")
                .header(CLIENT_HEADER, "extension")
                .body(Body::from(body.to_owned()))
                .unwrap()
        };
        let body = r#"{"pageUrl":"https://files.example.com/","links":[
            {"url":"https://files.example.com/a.zip","text":"A"},
            {"url":"javascript:alert(1)"},
            {"url":"mailto:me@example.com"},
            {"url":"https://cdn.example.net/b.mp4"}]}"#;
        let response = bridge.clone().router().oneshot(post(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let got = pages.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0]
                .links
                .iter()
                .map(|l| l.url.as_str())
                .collect::<Vec<_>>(),
            [
                "https://files.example.com/a.zip",
                "https://cdn.example.net/b.mp4"
            ]
        );

        let none = r#"{"pageUrl":"https://files.example.com/","links":[{"url":"ftp://x/y"}]}"#;
        let response = bridge.clone().router().oneshot(post(none)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let page = r#"{"pageUrl":"file:///etc/","links":[{"url":"https://x.org/a"}]}"#;
        let response = bridge.router().oneshot(post(page)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    fn post(origin: Option<&str>, client: bool, body: &str) -> Request<Body> {
        let mut request = Request::post("/api/download").header("content-type", "application/json");
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        if client {
            request = request
                .header(CLIENT_HEADER, "extension")
                .header(BROWSER_HEADER, "Chrome");
        }
        request.body(Body::from(body.to_owned())).unwrap()
    }

    const BODY: &str =
        r#"{"url":"https://example.com/a.zip","referrer":"https://example.com/","cookies":"s=1"}"#;

    #[tokio::test]
    async fn accepts_downloads_from_extensions() {
        let (bridge, received) = bridge(true);
        let response = bridge
            .clone()
            .router()
            .oneshot(post(Some("chrome-extension://abcdef"), true, BODY))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let received = received.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(
            received[0].headers(),
            [
                ("Referer".to_owned(), "https://example.com/".to_owned()),
                ("Cookie".to_owned(), "s=1".to_owned()),
            ]
        );
        assert!(bridge.connected().contains_key("chrome"));
    }

    #[tokio::test]
    async fn rejects_web_pages() {
        let (bridge, received) = bridge(true);
        for request in [
            post(Some("https://evil.example"), true, BODY),
            post(Some("chrome-extension://abcdef"), false, BODY),
            post(None, false, BODY),
        ] {
            let response = bridge.clone().router().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        assert!(received.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn never_approves_cors_preflights() {
        let (bridge, _) = bridge(true);
        let request = Request::builder()
            .method("OPTIONS")
            .uri("/api/download")
            .header("origin", "https://evil.example")
            .header("access-control-request-method", "POST")
            .header("access-control-request-headers", CLIENT_HEADER)
            .body(Body::empty())
            .unwrap();
        let response = bridge.router().oneshot(request).await.unwrap();
        assert!(!response.status().is_success());
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-origin")
        );
    }

    #[tokio::test]
    async fn respects_disabled_integration_and_bad_links() {
        let (bridge, received) = bridge(false);
        let response = bridge
            .router()
            .oneshot(post(None, true, BODY))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);

        let (bridge, _) = self::bridge(true);
        let response = bridge
            .router()
            .oneshot(post(None, true, r#"{"url":"file:///etc/passwd"}"#))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(received.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn accepts_magnet_links() {
        let (bridge, received) = bridge(true);
        let body = r#"{"url":"magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567"}"#;
        let response = bridge
            .router()
            .oneshot(post(None, true, body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert!(received.lock().unwrap()[0].url.starts_with("magnet:?"));
    }

    #[tokio::test]
    async fn ping_reports_configuration() {
        let (bridge, _) = bridge(true);
        let request = Request::get("/api/ping")
            .header(CLIENT_HEADER, "extension")
            .body(Body::empty())
            .unwrap();
        let response = bridge.router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["app"], "sdm");
        assert_eq!(json["enabled"], true);
        assert_eq!(json["captureExtensions"][0], "zip");
    }

    #[tokio::test]
    async fn accepts_videos_from_extensions() {
        let videos = Arc::new(Mutex::new(Vec::new()));
        let bridge = Bridge::new(Recorder {
            enabled: true,
            videos: videos.clone(),
            ..Recorder::default()
        });
        let body = r#"{"pageUrl":"https://www.youtube.com/watch?v=x","title":"Clip",
            "cookies":[{"name":"SID","value":"1","domain":".youtube.com","secure":true}]}"#;
        let request = |body: &str| {
            Request::post("/api/media")
                .header("content-type", "application/json")
                .header(CLIENT_HEADER, "extension")
                .body(Body::from(body.to_owned()))
                .unwrap()
        };
        let response = bridge
            .clone()
            .router()
            .oneshot(request(body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let response = bridge
            .router()
            .oneshot(request(r#"{"pageUrl":"javascript:alert(1)"}"#))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let videos = videos.lock().unwrap();
        assert_eq!(videos.len(), 1);
        assert_eq!(videos[0].cookies[0].path, "/");
        assert_eq!(videos[0].title.as_deref(), Some("Clip"));
    }

    #[tokio::test]
    async fn reads_and_adds_videos_for_the_page_menu() {
        let (bridge, _) = bridge(true);
        let post = |path: &str, body: &str| {
            Request::post(path)
                .header("content-type", "application/json")
                .header(CLIENT_HEADER, "extension")
                .body(Body::from(body.to_owned()))
                .unwrap()
        };
        let read = |response: Response| async move {
            let status = response.status();
            let body = axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap();
            (status, String::from_utf8_lossy(&body).into_owned())
        };
        let page = r#""pageUrl":"https://videos.example.com/watch/1""#;
        let (status, body) = read(
            bridge
                .clone()
                .router()
                .oneshot(post(
                    "/api/media/probe",
                    &format!("{{{page},\"wait\":true}}"),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains(r#""title":"Clip""#), "{body}");

        let response = bridge
            .clone()
            .router()
            .oneshot(post("/api/media/probe", &format!("{{{page}}}")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let (status, body) = read(
            bridge
                .clone()
                .router()
                .oneshot(post(
                    "/api/media/probe",
                    r#"{"pageUrl":"https://broken.example.com/","wait":true}"#,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, "Unsupported URL")
        );

        let (status, body) = read(
            bridge
                .clone()
                .router()
                .oneshot(post(
                    "/api/media/add",
                    &format!(r#"{{"source":{{{page}}},"option":"v720"}}"#),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!((status, body.as_str()), (StatusCode::OK, r#"{"id":7}"#));

        let response = bridge
            .router()
            .oneshot(
                Request::post("/api/media/add")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"source":{{{page}}},"option":"v720"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    async fn job_request(
        bridge: &Arc<Bridge>,
        route: &str,
        owner: &str,
        origin: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = bridge
            .clone()
            .router()
            .oneshot(
                Request::post(route)
                    .header(CLIENT_HEADER, "extension")
                    .header(BROWSER_HEADER, "chrome")
                    .header(header::ORIGIN, origin)
                    .header("x-sdm-probe-owner", owner)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
        )
    }

    fn probe_bridge() -> Arc<Bridge> {
        Bridge::new(Recorder {
            enabled: true,
            ..Default::default()
        })
    }

    #[tokio::test(start_paused = true)]
    async fn short_job_polls_keep_discovery_alive_beyond_thirty_seconds() {
        let bridge = probe_bridge();
        let owner = "extension:17:2:document";
        let origin = "chrome-extension://extension";
        let (_, state) = job_request(&bridge, "/api/media/probe/start", owner, origin,
            serde_json::json!({"clientId":"long-job", "source":{"pageUrl":"https://slow.example.org/clip"}})).await;
        assert_eq!(state["state"], "pending");
        tokio::task::yield_now().await;
        for _ in 0..4 {
            tokio::time::advance(Duration::from_secs(10)).await;
            let (status, state) = job_request(
                &bridge,
                "/api/media/probe/status",
                owner,
                origin,
                serde_json::json!({"clientId":"long-job"}),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(state["state"], "pending");
        }
        tokio::time::advance(Duration::from_secs(6)).await;
        tokio::task::yield_now().await;
        let (_, state) = job_request(
            &bridge,
            "/api/media/probe/status",
            owner,
            origin,
            serde_json::json!({"clientId":"long-job"}),
        )
        .await;
        assert_eq!(state["state"], "done");
        assert_eq!(state["info"]["title"], "Clip");
    }

    #[tokio::test(start_paused = true)]
    async fn job_ownership_is_scoped_to_document_and_extension_origin() {
        let bridge = probe_bridge();
        let origin = "chrome-extension://extension";
        let start = serde_json::json!({"clientId":"shared-id", "source":{"pageUrl":"https://never.example.org/clip"}});
        for owner in ["extension:1:0:a", "extension:2:0:b"] {
            assert_eq!(
                job_request(
                    &bridge,
                    "/api/media/probe/start",
                    owner,
                    origin,
                    start.clone()
                )
                .await
                .0,
                StatusCode::OK
            );
        }
        let client = serde_json::json!({"clientId":"shared-id"});
        assert_eq!(
            job_request(
                &bridge,
                "/api/media/probe/cancel",
                "extension:1:0:a",
                origin,
                client.clone()
            )
            .await
            .0,
            StatusCode::ACCEPTED
        );
        let (_, first) = job_request(
            &bridge,
            "/api/media/probe/status",
            "extension:1:0:a",
            origin,
            client.clone(),
        )
        .await;
        let (_, second) = job_request(
            &bridge,
            "/api/media/probe/status",
            "extension:2:0:b",
            origin,
            client.clone(),
        )
        .await;
        assert_eq!(first["state"], "error");
        assert_eq!(second["state"], "pending");
        assert_eq!(
            job_request(
                &bridge,
                "/api/media/probe/status",
                "extension:2:0:b",
                "chrome-extension://unrelated",
                client.clone()
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            job_request(
                &bridge,
                "/api/media/probe/cancel",
                "extension:2:0:b",
                "https://web.example",
                client
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test(start_paused = true)]
    async fn abandoned_jobs_expire_and_cancel_before_start_is_remembered() {
        let bridge = probe_bridge();
        let owner = "extension:17:0:document";
        let origin = "chrome-extension://extension";
        let client = serde_json::json!({"clientId":"closed-job"});
        assert_eq!(
            job_request(&bridge, "/api/media/probe/cancel", owner, origin, client)
                .await
                .0,
            StatusCode::ACCEPTED
        );
        let (_, state) = job_request(&bridge, "/api/media/probe/start", owner, origin,
            serde_json::json!({"clientId":"closed-job", "source":{"pageUrl":"https://never.example.org/clip"}})).await;
        assert_eq!(state["state"], "error");
        assert!(state["error"].as_str().unwrap().contains("cancelled"));
        job_request(&bridge, "/api/media/probe/start", owner, origin,
            serde_json::json!({"clientId":"abandoned-job", "source":{"pageUrl":"https://never.example.org/clip"}})).await;
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(26)).await;
        tokio::task::yield_now().await;
        let (status, state) = job_request(
            &bridge,
            "/api/media/probe/status",
            owner,
            origin,
            serde_json::json!({"clientId":"abandoned-job"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(state["state"], "error");
        assert!(state["error"].as_str().unwrap().contains("expired"));
    }

    #[tokio::test(start_paused = true)]
    async fn repeated_start_deduplicates_but_cannot_replace_authentication_context() {
        let bridge = probe_bridge();
        let owner = "extension:17:0:document";
        let origin = "chrome-extension://extension";
        let start = serde_json::json!({"clientId":"repeat-job", "source":{"pageUrl":"https://never.example.org/clip"}});
        for _ in 0..2 {
            assert_eq!(
                job_request(
                    &bridge,
                    "/api/media/probe/start",
                    owner,
                    origin,
                    start.clone()
                )
                .await
                .0,
                StatusCode::OK
            );
        }
        assert_eq!(bridge.probes.lock().unwrap().len(), 1);
        let changed = serde_json::json!({"clientId":"repeat-job", "source":{"pageUrl":"https://never.example.org/clip", "userAgent":"different profile"}});
        assert_eq!(
            job_request(&bridge, "/api/media/probe/start", owner, origin, changed)
                .await
                .0,
            StatusCode::CONFLICT
        );
    }
}
