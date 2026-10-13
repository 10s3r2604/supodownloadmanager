//! Turns what yt-dlp knows about a video into a short list of choices:
//! 1080p, 720p, ..., audio only.

use std::cmp::Reverse;

use serde::{Deserialize, Serialize};

/// What yt-dlp found on a page, ready for the quality picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub title: String,
    pub thumbnail: Option<String>,
    /// Length in seconds.
    pub duration: Option<f64>,
    /// The site, e.g. `Youtube`.
    pub site: String,
    /// The page of the video.
    pub url: String,
    pub options: Vec<MediaOption>,
    /// Id of the option picked by default.
    pub default: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Video,
    Audio,
}

/// One way to download the video.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaOption {
    pub id: String,
    pub kind: MediaKind,
    pub height: Option<u32>,
    pub fps: Option<u32>,
    /// File extension of the result.
    pub ext: String,
    /// Readable codec names, e.g. `H.264` and `AAC`.
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    /// Estimated size in bytes.
    pub size: Option<u64>,
    /// Video and audio are merged, or the audio converted: needs ffmpeg.
    pub needs_ffmpeg: bool,
    /// yt-dlp format selector.
    pub format: String,
    /// Container for merged video and audio.
    pub merge: Option<String>,
    /// Audio format to convert to.
    pub audio: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Info {
    #[serde(rename = "_type")]
    kind: Option<String>,
    title: Option<String>,
    thumbnail: Option<String>,
    duration: Option<f64>,
    extractor_key: Option<String>,
    webpage_url: Option<String>,
    ext: Option<String>,
    height: Option<u32>,
    tbr: Option<f64>,
    filesize: Option<f64>,
    filesize_approx: Option<f64>,
    #[serde(default)]
    formats: Vec<Format>,
    #[serde(default)]
    entries: Vec<Info>,
}

#[derive(Debug, Clone, Deserialize)]
struct Format {
    format_id: String,
    ext: Option<String>,
    vcodec: Option<String>,
    acodec: Option<String>,
    height: Option<u32>,
    fps: Option<f64>,
    filesize: Option<f64>,
    filesize_approx: Option<f64>,
    tbr: Option<f64>,
    abr: Option<f64>,
}

impl Format {
    /// `None` means unknown, which usually is a plain file with both.
    fn video(&self) -> bool {
        self.vcodec.as_deref() != Some("none")
    }

    fn audio(&self) -> bool {
        self.acodec.as_deref() != Some("none")
    }

    fn video_only(&self) -> bool {
        self.video() && self.acodec.as_deref() == Some("none")
    }

    fn audio_only(&self) -> bool {
        self.vcodec.as_deref() == Some("none") && self.audio()
    }

    fn ext(&self) -> &str {
        self.ext.as_deref().unwrap_or("mp4")
    }

    /// Storyboards and other things that are not media.
    fn junk(&self) -> bool {
        (!self.video() && !self.audio()) || matches!(self.ext(), "mhtml" | "jpg" | "png" | "webp")
    }

    fn size(&self, duration: Option<f64>) -> Option<u64> {
        self.filesize
            .filter(|s| s.is_finite() && *s > 0.0)
            .or(self.filesize_approx.filter(|s| s.is_finite() && *s > 0.0))
            .or_else(|| {
                Some(
                    self.tbr.or(self.abr)? * 1000.0 / 8.0
                        * duration.filter(|d| d.is_finite() && *d > 0.0)?,
                )
            })
            .filter(|s| s.is_finite() && *s > 0.0)
            .map(|s| s as u64)
    }

    fn bitrate(&self) -> f64 {
        self.tbr.or(self.abr).unwrap_or(0.0)
    }
}

/// How well a video codec plays everywhere: H.264 first.
fn video_rank(codec: Option<&str>) -> u8 {
    match codec.unwrap_or_default() {
        c if c.starts_with("avc") || c.starts_with("h264") => 3,
        c if c.starts_with("vp9") || c.starts_with("vp09") => 2,
        c if c.starts_with("av01") => 1,
        _ => 0,
    }
}

fn video_name(codec: Option<&str>) -> Option<String> {
    let codec = codec.filter(|c| *c != "none")?;
    Some(
        match codec {
            c if c.starts_with("avc") || c.starts_with("h264") => "H.264",
            c if c.starts_with("vp9") || c.starts_with("vp09") => "VP9",
            c if c.starts_with("av01") => "AV1",
            c if c.starts_with("hev") || c.starts_with("hvc") => "H.265",
            c => c.split('.').next().unwrap_or(c),
        }
        .to_owned(),
    )
}

fn audio_name(codec: Option<&str>) -> Option<String> {
    let codec = codec.filter(|c| *c != "none")?;
    Some(
        match codec {
            c if c.starts_with("mp4a") || c.starts_with("aac") => "AAC",
            c if c.starts_with("opus") => "Opus",
            c if c.starts_with("mp3") => "MP3",
            c if c.starts_with("vorbis") => "Vorbis",
            c => c.split('.').next().unwrap_or(c),
        }
        .to_owned(),
    )
}

/// Reads the output of `yt-dlp -J`.
pub fn parse(json: &str) -> Result<MediaInfo, String> {
    let mut info: Info =
        serde_json::from_str(json).map_err(|e| format!("unexpected yt-dlp output: {e}"))?;
    if info.kind.as_deref() == Some("playlist") {
        if info.entries.is_empty() {
            return Err("no video found on this page".into());
        }
        info = info.entries.swap_remove(0);
    }
    let site = info.extractor_key.clone().unwrap_or_default();
    let mut title = info.title.clone().unwrap_or_else(|| "video".into());
    // Videos embedded in a page are numbered: "Page title (1)".
    if site == "HTML5MediaEmbed"
        && let Some(stripped) = title.strip_suffix(" (1)")
    {
        title = stripped.to_owned();
    }
    let options = options(&info);
    let default = options
        .iter()
        .find(|o| o.kind == MediaKind::Video && o.height.is_none_or(|h| h <= 1080))
        .or(options.first())
        .map(|o| o.id.clone());
    Ok(MediaInfo {
        title,
        thumbnail: info.thumbnail.clone(),
        duration: info.duration,
        site,
        url: info.webpage_url.clone().unwrap_or_default(),
        options,
        default,
    })
}

fn options(info: &Info) -> Vec<MediaOption> {
    let duration = info.duration;
    let formats: Vec<&Format> = info.formats.iter().filter(|f| !f.junk()).collect();
    if formats.is_empty() {
        // A single file without a format list.
        let size = info
            .filesize
            .filter(|s| s.is_finite() && *s > 0.0)
            .or(info.filesize_approx.filter(|s| s.is_finite() && *s > 0.0))
            .or_else(|| {
                Some(
                    info.tbr.filter(|rate| rate.is_finite() && *rate > 0.0)? * 1000.0 / 8.0
                        * duration.filter(|d| d.is_finite() && *d > 0.0)?,
                )
            })
            .filter(|s| s.is_finite() && *s > 0.0)
            .map(|s| s as u64);
        let mut option = best(info.ext.as_deref().unwrap_or("mp4"), size, None);
        option.height = info.height.filter(|height| *height > 0);
        return vec![option];
    }

    let audios: Vec<&Format> = formats.iter().copied().filter(|f| f.audio_only()).collect();
    let pick_audio = |prefer_m4a: bool| {
        audios.iter().copied().max_by(|a, b| {
            let key = |f: &Format| ((f.ext() == "m4a") == prefer_m4a, f.bitrate());
            key(a)
                .partial_cmp(&key(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    };

    let mut heights: Vec<u32> = formats
        .iter()
        .filter(|f| f.video())
        .filter_map(|f| f.height)
        .collect();
    heights.sort_unstable_by_key(|&h| Reverse(h));
    heights.dedup();

    let mut out = Vec::new();
    for &height in &heights {
        let at: Vec<&Format> = formats
            .iter()
            .copied()
            .filter(|f| f.video() && f.height == Some(height))
            .collect();
        let fps = |f: &Format| f.fps.map(|v| v.round() as u32).filter(|&v| v > 0);
        let fallback = format!("bv*[height<={height}]+ba/b[height<={height}]/b");

        // A file that already has sound needs no merging.
        if let Some(both) = best_of(at.iter().copied().filter(|f| !f.video_only()).collect()) {
            out.push(MediaOption {
                id: format!("v{height}"),
                kind: MediaKind::Video,
                height: Some(height),
                fps: fps(both),
                ext: both.ext().to_owned(),
                vcodec: video_name(both.vcodec.as_deref()),
                acodec: audio_name(both.acodec.as_deref()),
                size: both.size(duration),
                needs_ffmpeg: false,
                format: format!("{}/{fallback}", both.format_id),
                merge: None,
                audio: None,
            });
            continue;
        }
        let Some(video) = best_of(at) else { continue };
        let Some(audio) = pick_audio(video.ext() == "mp4") else {
            continue;
        };
        let merge = match (video.ext(), audio.ext()) {
            ("mp4", "m4a" | "mp4") => "mp4",
            ("webm", "webm") => "webm",
            _ => "mkv",
        };
        out.push(MediaOption {
            id: format!("v{height}"),
            kind: MediaKind::Video,
            height: Some(height),
            fps: fps(video),
            ext: merge.to_owned(),
            vcodec: video_name(video.vcodec.as_deref()),
            acodec: audio_name(audio.acodec.as_deref()),
            size: video
                .size(duration)
                .zip(audio.size(duration))
                .map(|(v, a)| v + a),
            needs_ffmpeg: true,
            format: format!("{}+{}/{fallback}", video.format_id, audio.format_id),
            merge: Some(merge.to_owned()),
            audio: None,
        });
    }

    if out.is_empty() && formats.iter().any(|f| f.video()) {
        // Videos without a known height.
        let both = formats.iter().copied().find(|f| f.video() && f.audio());
        out.push(best(
            both.map_or("mp4", Format::ext),
            both.and_then(|f| f.size(duration)),
            both.map(|f| f.format_id.as_str()),
        ));
    }

    if let Some(m4a) = audios
        .iter()
        .copied()
        .filter(|f| f.ext() == "m4a")
        .max_by(|a, b| {
            a.bitrate()
                .partial_cmp(&b.bitrate())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    {
        out.push(MediaOption {
            id: "m4a".into(),
            kind: MediaKind::Audio,
            height: None,
            fps: None,
            ext: "m4a".into(),
            vcodec: None,
            acodec: audio_name(m4a.acodec.as_deref()),
            size: m4a.size(duration),
            needs_ffmpeg: false,
            format: format!("{}/ba[ext=m4a]/ba", m4a.format_id),
            merge: None,
            audio: None,
        });
    }
    out.push(MediaOption {
        id: "mp3".into(),
        kind: MediaKind::Audio,
        height: None,
        fps: None,
        ext: "mp3".into(),
        vcodec: None,
        acodec: Some("MP3".into()),
        // About 190 kbit/s.
        size: duration.map(|d| (d * 24_000.0) as u64),
        needs_ffmpeg: true,
        format: "ba/b".into(),
        merge: None,
        audio: Some("mp3".into()),
    });
    out
}

/// The video that plays most widely, then the one with the most detail.
fn best_of(list: Vec<&Format>) -> Option<&Format> {
    list.into_iter().max_by(|a, b| {
        let key = |f: &Format| (video_rank(f.vcodec.as_deref()), f.bitrate());
        key(a)
            .partial_cmp(&key(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

fn best(ext: &str, size: Option<u64>, id: Option<&str>) -> MediaOption {
    MediaOption {
        id: "best".into(),
        kind: MediaKind::Video,
        height: None,
        fps: None,
        ext: ext.to_owned(),
        vcodec: None,
        acodec: None,
        size,
        needs_ffmpeg: false,
        format: id.map_or_else(|| "b/bv*+ba".to_owned(), |id| format!("{id}/b/bv*+ba")),
        merge: None,
        audio: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(
        id: &str,
        ext: &str,
        vcodec: &str,
        acodec: &str,
        height: Option<u32>,
        size: u64,
    ) -> String {
        let height = height.map_or("null".into(), |h| h.to_string());
        format!(
            r#"{{"format_id":"{id}","ext":"{ext}","vcodec":"{vcodec}","acodec":"{acodec}","height":{height},"fps":30,"filesize":{size},"tbr":1000}}"#
        )
    }

    fn youtube() -> String {
        let formats = [
            format("sb0", "mhtml", "none", "none", Some(90), 0),
            format(
                "18",
                "mp4",
                "avc1.42001E",
                "mp4a.40.2",
                Some(360),
                9_000_000,
            ),
            format("134", "mp4", "avc1.4d401e", "none", Some(360), 6_000_000),
            format("136", "mp4", "avc1.4d401f", "none", Some(720), 20_000_000),
            format("137", "mp4", "avc1.640028", "none", Some(1080), 40_000_000),
            format("248", "webm", "vp9", "none", Some(1080), 35_000_000),
            format("313", "webm", "vp9", "none", Some(2160), 200_000_000),
            format("140", "m4a", "none", "mp4a.40.2", None, 3_000_000),
            format("251", "webm", "none", "opus", None, 3_500_000),
        ]
        .join(",");
        format!(
            r#"{{"_type":"video","title":"Clip","thumbnail":"https://i.ytimg.com/vi/x/hq.jpg","duration":200,"extractor_key":"Youtube","webpage_url":"https://www.youtube.com/watch?v=x","formats":[{formats}]}}"#
        )
    }

    #[test]
    fn offers_each_height_and_audio() {
        let info = parse(&youtube()).unwrap();
        let ids: Vec<&str> = info.options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["v2160", "v1080", "v720", "v360", "m4a", "mp3"]);
        assert_eq!(info.default.as_deref(), Some("v1080"));
        assert_eq!(info.site, "Youtube");

        let by_id = |id: &str| info.options.iter().find(|o| o.id == id).unwrap();
        // H.264 is preferred, merged with AAC into MP4.
        let full_hd = by_id("v1080");
        assert!(full_hd.format.starts_with("137+140/"));
        assert_eq!(full_hd.merge.as_deref(), Some("mp4"));
        assert_eq!(full_hd.size, Some(43_000_000));
        assert!(full_hd.needs_ffmpeg);
        assert_eq!(full_hd.vcodec.as_deref(), Some("H.264"));
        // Only VP9 exists in 4K: merged with Opus into WebM.
        assert_eq!(by_id("v2160").merge.as_deref(), Some("webm"));
        assert!(by_id("v2160").format.starts_with("313+251/"));
        // 360p already has sound.
        let low = by_id("v360");
        assert!(low.format.starts_with("18/"));
        assert!(!low.needs_ffmpeg);
        assert_eq!(low.acodec.as_deref(), Some("AAC"));

        assert!(by_id("m4a").format.starts_with("140/"));
        assert!(!by_id("m4a").needs_ffmpeg);
        assert_eq!(by_id("mp3").audio.as_deref(), Some("mp3"));
        assert!(by_id("mp3").needs_ffmpeg);
    }

    #[test]
    fn handles_plain_files_and_embedded_videos() {
        let json = r#"{"_type":"video","title":"Trailer (1)","extractor_key":"HTML5MediaEmbed","webpage_url":"http://x/watch","ext":"mp4",
            "formats":[{"format_id":"0","ext":"mp4","vcodec":null,"acodec":null}]}"#;
        let info = parse(json).unwrap();
        assert_eq!(info.title, "Trailer");
        let ids: Vec<&str> = info.options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["best", "mp3"]);
        assert_eq!(info.options[0].format, "0/b/bv*+ba");
        assert_eq!(info.default.as_deref(), Some("best"));
    }

    #[test]
    fn estimates_sizes_from_bitrates() {
        let json = r#"{"title":"Stream","duration":20,"extractor_key":"Generic","formats":[
            {"format_id":"2","ext":"m4a","vcodec":"none","acodec":"mp4a.40.2","tbr":128},
            {"format_id":"0","ext":"mp4","vcodec":"avc1.64001f","acodec":"none","height":720,"tbr":2000}]}"#;
        let info = parse(json).unwrap();
        let hd = &info.options[0];
        assert_eq!(hd.id, "v720");
        assert_eq!(hd.size, Some(5_000_000 + 320_000));
        assert_eq!(hd.merge.as_deref(), Some("mp4"));
    }

    #[test]
    fn plain_file_metadata_keeps_its_measured_height_and_estimates_size() {
        let info = parse(r#"{"title":"Direct video","ext":"mp4","height":1080,"duration":20,"tbr":2048,"filesize":0}"#).unwrap();
        assert_eq!(info.options[0].height, Some(1080));
        assert_eq!(info.options[0].size, Some(5_120_000));
        let invalid = parse(r#"{"ext":"mp4","duration":-20,"tbr":-2048,"filesize":0}"#).unwrap();
        assert_eq!(invalid.options[0].size, None);
    }

    #[test]
    fn takes_the_first_video_of_a_playlist() {
        let json = format!(
            r#"{{"_type":"playlist","title":"List","entries":[{}]}}"#,
            youtube()
        );
        assert_eq!(parse(&json).unwrap().title, "Clip");
        assert!(parse(r#"{"_type":"playlist","entries":[]}"#).is_err());
    }
}
