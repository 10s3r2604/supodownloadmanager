//! A small HTTP server on `127.0.0.1` that the browser extension talks to.
//!
//! Only browser extensions may use it. Every request must carry the
//! `X-SDM-Client` header: a web page can only send such a header after a CORS
//! preflight, which this server never approves. Requests whose `Origin` is a
//! web page are rejected as well.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::media::MediaSource;

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

/// What the extension needs to know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeConfig {
    pub enabled: bool,
    pub capture_extensions: Vec<String>,
}

/// What the app does with requests from the extension.
pub trait Handler: Send + Sync + 'static {
    fn config(&self) -> BridgeConfig;
    fn download(&self, download: BrowserDownload);
    /// A video on a web page, to download with yt-dlp.
    fn media(&self, source: MediaSource);
}

pub struct Bridge {
    handler: Box<dyn Handler>,
    seen: Mutex<HashMap<String, Instant>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Ping {
    app: &'static str,
    version: &'static str,
    /// The app downloads videos from web pages (`/api/media`).
    media: bool,
    /// The app takes magnet links in `/api/download`.
    torrent: bool,
    #[serde(flatten)]
    config: BridgeConfig,
}

impl Bridge {
    pub fn new(handler: impl Handler) -> Arc<Self> {
        Arc::new(Self {
            handler: Box::new(handler),
            seen: Mutex::new(HashMap::new()),
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
            .layer(DefaultBodyLimit::max(512 * 1024))
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
    Json(Ping {
        app: "sdm",
        version: env!("CARGO_PKG_VERSION"),
        media: true,
        torrent: true,
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

async fn media(
    State(bridge): State<Arc<Bridge>>,
    headers: HeaderMap,
    Json(source): Json<MediaSource>,
) -> Response {
    if !from_extension(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    bridge.remember(&headers);
    if !bridge.handler.config().enabled {
        return (StatusCode::CONFLICT, "browser integration is turned off").into_response();
    }
    let valid =
        url::Url::parse(&source.page_url).is_ok_and(|url| matches!(url.scheme(), "http" | "https"));
    if !valid {
        return (
            StatusCode::BAD_REQUEST,
            "only http and https pages are supported",
        )
            .into_response();
    }
    bridge.handler.media(source);
    StatusCode::ACCEPTED.into_response()
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;

    #[derive(Default)]
    struct Recorder {
        enabled: bool,
        received: Arc<Mutex<Vec<BrowserDownload>>>,
        videos: Arc<Mutex<Vec<MediaSource>>>,
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
    }

    fn bridge(enabled: bool) -> (Arc<Bridge>, Arc<Mutex<Vec<BrowserDownload>>>) {
        let received = Arc::new(Mutex::new(Vec::new()));
        let bridge = Bridge::new(Recorder {
            enabled,
            received: received.clone(),
            videos: Arc::default(),
        });
        (bridge, received)
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
            received: Arc::default(),
            videos: videos.clone(),
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
}
